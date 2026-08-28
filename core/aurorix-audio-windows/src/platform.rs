//! Windows x64 COM/WASAPI Shared and Exclusive implementation.

use std::ptr;

use aurorix_audio::{
    format::{AudioFormat, FormatError, SampleFormat},
    output_report::{
        ChannelMappingStatus, FormatConversionStatus, OutputObservation, OutputRequest,
        PlaybackRate, ResamplingStatus, Volume,
    },
};
use windows::{
    Win32::{
        Foundation::{CloseHandle, S_FALSE, S_OK, WAIT_FAILED, WAIT_OBJECT_0},
        Media::{
            Audio::{
                AUDCLNT_E_DEVICE_INVALIDATED, AUDCLNT_SHAREMODE_EXCLUSIVE,
                AUDCLNT_SHAREMODE_SHARED, AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM,
                AUDCLNT_STREAMFLAGS_EVENTCALLBACK, AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY,
                DEVICE_STATE, DEVICE_STATE_ACTIVE, DEVICE_STATE_DISABLED, DEVICE_STATE_NOTPRESENT,
                DEVICE_STATE_UNPLUGGED, IAudioClient, IAudioRenderClient, IMMDevice,
                IMMDeviceEnumerator, MMDeviceEnumerator, WAVE_FORMAT_PCM, WAVEFORMATEX,
                WAVEFORMATEXTENSIBLE,
            },
            KernelStreaming::WAVE_FORMAT_EXTENSIBLE,
            Multimedia::WAVE_FORMAT_IEEE_FLOAT,
        },
        System::{
            Com::{
                CLSCTX_ALL, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx, CoTaskMemFree,
                CoUninitialize,
            },
            Threading::{CreateEventW, INFINITE, WaitForSingleObject},
        },
    },
    core::{GUID, PCWSTR},
};

use crate::{
    DeviceEnumerationError, DeviceId, DeviceInfo, DeviceOperation, DeviceSelector, DeviceState,
    ExclusiveBackendFactory, ExclusiveFormatSupport, ExclusiveOpenError, ExclusiveOpenOutput,
    ExclusiveOutputConfig, ExclusiveOutputError, FallbackPolicy, OutputFallback,
    RenderBackendError, RenderEvent, RenderOperation, SharedOutput, SharedOutputConfig,
    SharedOutputError, SharedRenderBackend, open_exclusive_with_shared_fallback,
};

const PCM_SUBTYPE: GUID = GUID::from_u128(0x00000001_0000_0010_8000_00aa00389b71);
const IEEE_FLOAT_SUBTYPE: GUID = GUID::from_u128(0x00000003_0000_0010_8000_00aa00389b71);

/// A COM apartment owned by the thread that creates and uses a WASAPI client.
#[derive(Debug)]
struct ComApartment {
    initialized: bool,
}

impl ComApartment {
    fn initialize() -> Result<Self, DeviceEnumerationError> {
        // WASAPI resources stay on the dedicated worker thread. An existing
        // STA cannot be silently changed into the MTA required by this path.
        let result = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
        if result == S_OK || result == S_FALSE {
            Ok(Self { initialized: true })
        } else {
            Err(DeviceEnumerationError::Api {
                operation: DeviceOperation::InitializeCom,
                hresult: result.0,
            })
        }
    }
}

impl Drop for ComApartment {
    fn drop(&mut self) {
        if self.initialized {
            unsafe { CoUninitialize() };
        }
    }
}

/// Owns a format buffer returned through the COM task allocator.
#[derive(Debug)]
struct CoTaskMemWaveFormat(*mut WAVEFORMATEX);

impl Default for CoTaskMemWaveFormat {
    fn default() -> Self {
        Self(ptr::null_mut())
    }
}

impl CoTaskMemWaveFormat {
    fn out_ptr(&mut self) -> *mut *mut WAVEFORMATEX {
        &raw mut self.0
    }

    fn set(&mut self, pointer: *mut WAVEFORMATEX) {
        debug_assert!(self.0.is_null());
        self.0 = pointer;
    }

    fn as_ptr(&self) -> *const WAVEFORMATEX {
        self.0.cast_const()
    }

    fn is_null(&self) -> bool {
        self.0.is_null()
    }
}

impl Drop for CoTaskMemWaveFormat {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe { CoTaskMemFree(Some(self.0.cast())) };
        }
    }
}

/// The system `MMDevice` enumerator. Create and consume it on one worker
/// thread; it owns the COM apartment needed by endpoint activation.
#[derive(Debug)]
pub struct WasapiDeviceEnumerator {
    apartment: Option<ComApartment>,
    enumerator: IMMDeviceEnumerator,
}

impl WasapiDeviceEnumerator {
    /// Creates an MTA-bound endpoint enumerator.
    ///
    /// # Errors
    ///
    /// Returns a typed error when COM or the `MMDevice` enumerator cannot be
    /// initialized.
    pub fn new() -> Result<Self, DeviceEnumerationError> {
        let apartment = ComApartment::initialize()?;
        let enumerator = unsafe {
            CoCreateInstance::<_, IMMDeviceEnumerator>(&MMDeviceEnumerator, None, CLSCTX_ALL)
        }
        .map_err(|error| DeviceEnumerationError::Api {
            operation: DeviceOperation::CreateEnumerator,
            hresult: error.code().0,
        })?;
        Ok(Self {
            apartment: Some(apartment),
            enumerator,
        })
    }

    /// Enumerates render endpoints, including inactive devices for a stable
    /// control-plane list. Only active devices can be opened.
    ///
    /// # Errors
    ///
    /// Returns a typed error when Windows cannot enumerate or inspect an
    /// endpoint.
    pub fn enumerate(&self) -> Result<Vec<DeviceInfo>, DeviceEnumerationError> {
        let devices = unsafe {
            self.enumerator.EnumAudioEndpoints(
                windows::Win32::Media::Audio::eRender,
                DEVICE_STATE(
                    DEVICE_STATE_ACTIVE.0
                        | DEVICE_STATE_DISABLED.0
                        | DEVICE_STATE_NOTPRESENT.0
                        | DEVICE_STATE_UNPLUGGED.0,
                ),
            )
        }
        .map_err(|error| DeviceEnumerationError::Api {
            operation: DeviceOperation::Enumerate,
            hresult: error.code().0,
        })?;
        let count = unsafe { devices.GetCount() }.map_err(|error| DeviceEnumerationError::Api {
            operation: DeviceOperation::Enumerate,
            hresult: error.code().0,
        })?;
        let mut output = Vec::with_capacity(count as usize);
        for index in 0..count {
            let device =
                unsafe { devices.Item(index) }.map_err(|error| DeviceEnumerationError::Api {
                    operation: DeviceOperation::Enumerate,
                    hresult: error.code().0,
                })?;
            output.push(Self::device_info(&device, false)?);
        }
        let default_id = self.default_device().ok().map(|device| device.id().clone());
        if let Some(default_id) = default_id {
            for device in &mut output {
                device.is_default = device.id() == &default_id;
            }
        }
        Ok(output)
    }

    /// Resolves the current default render endpoint.
    ///
    /// # Errors
    ///
    /// Returns a typed error when Windows cannot resolve or inspect the
    /// default endpoint.
    pub fn default_device(&self) -> Result<DeviceInfo, DeviceEnumerationError> {
        let device = unsafe {
            self.enumerator.GetDefaultAudioEndpoint(
                windows::Win32::Media::Audio::eRender,
                windows::Win32::Media::Audio::eConsole,
            )
        }
        .map_err(|error| DeviceEnumerationError::Api {
            operation: DeviceOperation::ResolveDefault,
            hresult: error.code().0,
        })?;
        Self::device_info(&device, true)
    }

    /// Resolves an endpoint by its opaque id.
    ///
    /// # Errors
    ///
    /// Returns a typed error when the id does not resolve to an inspectable
    /// endpoint.
    pub fn device(&self, id: &DeviceId) -> Result<DeviceInfo, DeviceEnumerationError> {
        let wide: Vec<u16> = id.as_str().encode_utf16().chain(Some(0)).collect();
        let device =
            unsafe { self.enumerator.GetDevice(PCWSTR(wide.as_ptr())) }.map_err(|error| {
                DeviceEnumerationError::Api {
                    operation: DeviceOperation::ResolveSelected,
                    hresult: error.code().0,
                }
            })?;
        Self::device_info(&device, false)
    }

    fn device_info(
        device: &IMMDevice,
        is_default: bool,
    ) -> Result<DeviceInfo, DeviceEnumerationError> {
        let raw_id = unsafe { device.GetId() }.map_err(|error| DeviceEnumerationError::Api {
            operation: DeviceOperation::ReadId,
            hresult: error.code().0,
        })?;
        if raw_id.is_null() {
            return Err(DeviceEnumerationError::InvalidDeviceId);
        }
        let id = unsafe { raw_id.to_string() }
            .map_err(|_| DeviceEnumerationError::InvalidDeviceId)
            .and_then(|value| {
                DeviceId::new(value).map_err(|_| DeviceEnumerationError::InvalidDeviceId)
            });
        unsafe { CoTaskMemFree(Some(raw_id.0.cast())) };
        let id = id?;
        let state = unsafe { device.GetState() }.map_err(|error| DeviceEnumerationError::Api {
            operation: DeviceOperation::ReadState,
            hresult: error.code().0,
        })?;
        let state = device_state(state);
        let name = format!("Windows audio endpoint {}", id.as_str());
        DeviceInfo::new(id.as_str(), name, state, is_default)
            .map_err(|_| DeviceEnumerationError::InvalidDeviceId)
    }

    fn take_apartment(&mut self) -> ComApartment {
        self.apartment
            .take()
            .expect("WASAPI apartment is present until activation")
    }
}

impl crate::DeviceEnumerator for WasapiDeviceEnumerator {
    fn enumerate(&mut self) -> Result<Vec<DeviceInfo>, DeviceEnumerationError> {
        Self::enumerate(self)
    }

    fn default_device(&mut self) -> Result<DeviceInfo, DeviceEnumerationError> {
        Self::default_device(self)
    }

    fn device(&mut self, id: &DeviceId) -> Result<DeviceInfo, DeviceEnumerationError> {
        Self::device(self, id)
    }
}

/// Enumerates current Windows render endpoints on a dedicated MTA apartment.
///
/// # Errors
///
/// Returns a typed error when COM or Windows endpoint discovery fails.
pub fn enumerate_devices() -> Result<Vec<DeviceInfo>, DeviceEnumerationError> {
    WasapiDeviceEnumerator::new()?.enumerate()
}

/// Opens an event-driven WASAPI Shared endpoint using the configured device
/// selector and fallback policy.
///
/// # Errors
///
/// Returns a typed output error when endpoint activation, format negotiation,
/// or Shared client initialization fails.
pub fn open_shared(
    config: &SharedOutputConfig,
) -> Result<SharedOutput<WasapiSharedBackend>, SharedOutputError> {
    let mut enumerator = WasapiDeviceEnumerator::new()
        .map_err(|error| map_enumerator_error(&error, config.fallback_policy()))?;
    let selected = match config.selector() {
        DeviceSelector::Default => enumerator.default_device(),
        DeviceSelector::Id(id) => enumerator.device(id),
    }
    .map_err(|error| map_enumerator_error(&error, config.fallback_policy()))?;
    if selected.state() != DeviceState::Active {
        let reason = match selected.state() {
            DeviceState::Disabled => crate::OutputUnavailableReason::DeviceDisabled,
            DeviceState::Unplugged => crate::OutputUnavailableReason::DeviceUnplugged,
            DeviceState::NotPresent => crate::OutputUnavailableReason::DeviceRemoved,
            DeviceState::Active => unreachable!(),
        };
        return Err(SharedOutputError::Unavailable {
            reason,
            fallback: fallback_for(config.fallback_policy(), OutputFallback::DefaultDevice),
        });
    }
    let backend = WasapiSharedBackend::open(
        enumerator.take_apartment(),
        selected,
        config.request(),
        config.period_frames(),
        config.fallback_policy(),
    )?;
    SharedOutput::from_backend(config, backend)
}

/// Opens an event-driven WASAPI Exclusive endpoint, retrying in Shared mode
/// only when the configured policy permits the typed fallback.
///
/// This is setup/control-plane work. Both variants return the same generic
/// render boundary, so no mode-specific allocation or blocking is introduced
/// into the realtime callback.
///
/// # Errors
///
/// Returns [`ExclusiveOpenError::Exclusive`] when Exclusive fails without an
/// allowed retry, or [`ExclusiveOpenError::SharedFallback`] when Shared retry
/// setup also fails.
pub fn open_exclusive(
    config: &ExclusiveOutputConfig,
) -> Result<ExclusiveOpenOutput<WasapiExclusiveBackend, WasapiSharedBackend>, ExclusiveOpenError> {
    let mut factory = match WasapiExclusiveFactory::new(config) {
        Ok(factory) => factory,
        Err(error) => return open_shared_for_exclusive_error(config, error),
    };
    open_exclusive_with_shared_fallback(&mut factory, config)
}

fn open_shared_for_exclusive_error(
    config: &ExclusiveOutputConfig,
    exclusive: ExclusiveOutputError,
) -> Result<ExclusiveOpenOutput<WasapiExclusiveBackend, WasapiSharedBackend>, ExclusiveOpenError> {
    if exclusive.fallback() != Some(OutputFallback::SharedMode) {
        return Err(ExclusiveOpenError::Exclusive(exclusive));
    }
    match open_shared(&config.shared_config()) {
        Ok(output) => Ok(ExclusiveOpenOutput::SharedFallback(output)),
        Err(shared) => Err(ExclusiveOpenError::SharedFallback { exclusive, shared }),
    }
}

fn map_enumerator_error(
    error: &DeviceEnumerationError,
    policy: FallbackPolicy,
) -> SharedOutputError {
    match error {
        DeviceEnumerationError::BackendUnavailable { .. }
        | DeviceEnumerationError::Api {
            operation: DeviceOperation::InitializeCom,
            ..
        }
        | DeviceEnumerationError::Api {
            operation: DeviceOperation::CreateEnumerator,
            ..
        } => SharedOutputError::Unavailable {
            reason: crate::OutputUnavailableReason::BackendUnavailable,
            fallback: None,
        },
        DeviceEnumerationError::Api { hresult, .. } => SharedOutputError::Backend {
            operation: RenderOperation::NegotiateFormat,
            hresult: Some(*hresult),
            fallback: fallback_for(policy, OutputFallback::DefaultDevice),
        },
        DeviceEnumerationError::InvalidDeviceId => {
            SharedOutputError::InvalidConfiguration { field: "device id" }
        }
    }
}

fn fallback_for(
    policy: FallbackPolicy,
    fallback: crate::OutputFallback,
) -> Option<crate::OutputFallback> {
    matches!(policy, FallbackPolicy::DefaultDevice).then_some(fallback)
}

fn exclusive_fallback_for(
    policy: FallbackPolicy,
    fallback: OutputFallback,
) -> Option<OutputFallback> {
    matches!(policy, FallbackPolicy::DefaultDevice).then_some(fallback)
}

fn map_exclusive_enumerator_error(
    error: &DeviceEnumerationError,
    policy: FallbackPolicy,
) -> ExclusiveOutputError {
    let fallback = exclusive_fallback_for(policy, OutputFallback::SharedMode);
    match error {
        DeviceEnumerationError::BackendUnavailable { .. }
        | DeviceEnumerationError::Api {
            operation: DeviceOperation::InitializeCom,
            ..
        }
        | DeviceEnumerationError::Api {
            operation: DeviceOperation::CreateEnumerator,
            ..
        } => ExclusiveOutputError::Unavailable {
            reason: crate::OutputUnavailableReason::BackendUnavailable,
            fallback,
        },
        DeviceEnumerationError::Api { hresult, .. } => ExclusiveOutputError::Backend {
            operation: RenderOperation::NegotiateFormat,
            hresult: Some(*hresult),
            fallback,
        },
        DeviceEnumerationError::InvalidDeviceId => {
            ExclusiveOutputError::InvalidConfiguration { field: "device id" }
        }
    }
}

fn activate_client(device: &DeviceInfo) -> Result<IAudioClient, i32> {
    let wide: Vec<u16> = device.id().as_str().encode_utf16().chain(Some(0)).collect();
    let enumerator = unsafe {
        CoCreateInstance::<_, IMMDeviceEnumerator>(&MMDeviceEnumerator, None, CLSCTX_ALL)
    }
    .map_err(|error| error.code().0)?;
    let device_handle =
        unsafe { enumerator.GetDevice(PCWSTR(wide.as_ptr())) }.map_err(|error| error.code().0)?;
    unsafe { device_handle.Activate::<IAudioClient>(CLSCTX_ALL, None) }
        .map_err(|error| error.code().0)
}

fn device_state(state: DEVICE_STATE) -> DeviceState {
    if state.0 & DEVICE_STATE_ACTIVE.0 != 0 {
        DeviceState::Active
    } else if state.0 & DEVICE_STATE_DISABLED.0 != 0 {
        DeviceState::Disabled
    } else if state.0 & DEVICE_STATE_UNPLUGGED.0 != 0 {
        DeviceState::Unplugged
    } else {
        DeviceState::NotPresent
    }
}

/// A fully initialized WASAPI Shared endpoint. It owns the COM apartment and
/// remains on the thread that created it.
#[derive(Debug)]
pub struct WasapiSharedBackend {
    _apartment: ComApartment,
    client: IAudioClient,
    render_client: IAudioRenderClient,
    event: EventHandle,
    device: DeviceInfo,
    format: AudioFormat,
    buffer_frames: usize,
    period_frames: usize,
    latency_frames: Option<u64>,
}

/// The concrete Shared output returned by the Windows adapter.
pub type WasapiSharedOutput = SharedOutput<WasapiSharedBackend>;

/// A fully initialized WASAPI Exclusive endpoint. It uses the same prepared
/// scratch/render boundary as [`WasapiSharedBackend`].
#[derive(Debug)]
pub struct WasapiExclusiveBackend {
    _apartment: ComApartment,
    client: IAudioClient,
    render_client: IAudioRenderClient,
    event: EventHandle,
    device: DeviceInfo,
    format: AudioFormat,
    buffer_frames: usize,
    period_frames: usize,
    latency_frames: Option<u64>,
}

/// The concrete Exclusive output returned by the Windows adapter.
pub type WasapiExclusiveOutput = SharedOutput<WasapiExclusiveBackend>;

/// The control-plane factory for a selected Windows endpoint.
#[derive(Debug)]
struct WasapiExclusiveFactory {
    apartment: Option<ComApartment>,
    device: DeviceInfo,
    fallback_policy: FallbackPolicy,
}

impl WasapiExclusiveFactory {
    fn new(config: &ExclusiveOutputConfig) -> Result<Self, ExclusiveOutputError> {
        let mut enumerator = WasapiDeviceEnumerator::new()
            .map_err(|error| map_exclusive_enumerator_error(&error, config.fallback_policy()))?;
        let device = match config.selector() {
            DeviceSelector::Default => enumerator.default_device(),
            DeviceSelector::Id(id) => enumerator.device(id),
        }
        .map_err(|error| map_exclusive_enumerator_error(&error, config.fallback_policy()))?;
        if device.state() != DeviceState::Active {
            let reason = match device.state() {
                DeviceState::Disabled => crate::OutputUnavailableReason::DeviceDisabled,
                DeviceState::Unplugged => crate::OutputUnavailableReason::DeviceUnplugged,
                DeviceState::NotPresent => crate::OutputUnavailableReason::DeviceRemoved,
                DeviceState::Active => unreachable!(),
            };
            return Err(ExclusiveOutputError::Unavailable {
                reason,
                fallback: exclusive_fallback_for(
                    config.fallback_policy(),
                    OutputFallback::SharedMode,
                ),
            });
        }
        Ok(Self {
            apartment: Some(enumerator.take_apartment()),
            device,
            fallback_policy: config.fallback_policy(),
        })
    }

    fn take_apartment(&mut self) -> ComApartment {
        self.apartment
            .take()
            .expect("WASAPI Exclusive apartment is present until activation")
    }
}

impl ExclusiveBackendFactory for WasapiExclusiveFactory {
    type ExclusiveBackend = WasapiExclusiveBackend;
    type SharedBackend = WasapiSharedBackend;

    fn exclusive_format_support(
        &mut self,
        requested: AudioFormat,
    ) -> Result<ExclusiveFormatSupport, ExclusiveOutputError> {
        let client =
            activate_client(&self.device).map_err(|hresult| ExclusiveOutputError::Backend {
                operation: RenderOperation::NegotiateFormat,
                hresult: Some(hresult),
                fallback: exclusive_fallback_for(self.fallback_policy, OutputFallback::SharedMode),
            })?;
        let requested_wave = wave_format(requested).map_err(|error| match error {
            SharedOutputError::InvalidConfiguration { field } => {
                ExclusiveOutputError::InvalidConfiguration { field }
            }
            _ => ExclusiveOutputError::InvalidConfiguration {
                field: "audio format",
            },
        })?;
        let support = unsafe {
            client.IsFormatSupported(AUDCLNT_SHAREMODE_EXCLUSIVE, &raw const requested_wave, None)
        };
        if support == S_OK {
            Ok(ExclusiveFormatSupport::Exact)
        } else {
            Err(ExclusiveOutputError::FormatNegotiation {
                requested,
                closest: None,
                fallback: exclusive_fallback_for(self.fallback_policy, OutputFallback::SharedMode),
            })
        }
    }

    fn open_exclusive(
        &mut self,
        config: &ExclusiveOutputConfig,
        negotiated_format: AudioFormat,
    ) -> Result<WasapiExclusiveOutput, ExclusiveOutputError> {
        let backend = WasapiExclusiveBackend::open(
            self.take_apartment(),
            self.device.clone(),
            negotiated_format,
            config.period_frames(),
            config.fallback_policy(),
        )?;
        let shared_config = config.shared_config();
        SharedOutput::from_backend(&shared_config, backend).map_err(|error| match error {
            SharedOutputError::InvalidConfiguration { field } => {
                ExclusiveOutputError::InvalidConfiguration { field }
            }
            SharedOutputError::Unavailable { reason, .. } => ExclusiveOutputError::Unavailable {
                reason,
                fallback: exclusive_fallback_for(
                    config.fallback_policy(),
                    OutputFallback::SharedMode,
                ),
            },
            SharedOutputError::FormatNegotiation {
                requested, closest, ..
            } => ExclusiveOutputError::FormatNegotiation {
                requested,
                closest,
                fallback: exclusive_fallback_for(
                    config.fallback_policy(),
                    OutputFallback::SharedMode,
                ),
            },
            SharedOutputError::Backend {
                operation, hresult, ..
            } => ExclusiveOutputError::Backend {
                operation,
                hresult,
                fallback: exclusive_fallback_for(
                    config.fallback_policy(),
                    OutputFallback::SharedMode,
                ),
            },
        })
    }

    fn open_shared(
        &mut self,
        config: &SharedOutputConfig,
    ) -> Result<WasapiSharedOutput, SharedOutputError> {
        crate::open_shared(config)
    }
}

impl WasapiSharedBackend {
    #[allow(clippy::too_many_lines)]
    fn open(
        apartment: ComApartment,
        device: DeviceInfo,
        request: aurorix_audio::output_report::OutputRequest,
        configured_period_frames: Option<usize>,
        fallback_policy: FallbackPolicy,
    ) -> Result<Self, SharedOutputError> {
        let client = activate_client(&device).map_err(|hresult| SharedOutputError::Backend {
            operation: RenderOperation::Initialize,
            hresult: Some(hresult),
            fallback: fallback_for(fallback_policy, OutputFallback::DefaultDevice),
        })?;
        let requested_format = request
            .requested_output_format()
            .unwrap_or(request.source_format());
        let requested_wave = wave_format(requested_format)?;
        let mut closest = CoTaskMemWaveFormat::default();
        let mut mix = CoTaskMemWaveFormat::default();
        let support = unsafe {
            client.IsFormatSupported(
                AUDCLNT_SHAREMODE_SHARED,
                &raw const requested_wave,
                Some(closest.out_ptr()),
            )
        };
        let (negotiated_format, format_ptr) = if support == S_OK {
            (requested_format, &raw const requested_wave)
        } else if support == S_FALSE && !closest.is_null() {
            let parsed = parse_wave_format(closest.as_ptr()).map_err(|_| {
                SharedOutputError::FormatNegotiation {
                    requested: requested_format,
                    closest: None,
                    fallback: fallback_for(fallback_policy, OutputFallback::SharedMixFormat),
                }
            })?;
            (parsed, closest.as_ptr())
        } else {
            mix.set(unsafe { client.GetMixFormat() }.map_err(|error| {
                SharedOutputError::Backend {
                    operation: RenderOperation::NegotiateFormat,
                    hresult: Some(error.code().0),
                    fallback: fallback_for(fallback_policy, OutputFallback::SharedMixFormat),
                }
            })?);
            let parsed = parse_wave_format(mix.as_ptr()).map_err(|_| {
                SharedOutputError::FormatNegotiation {
                    requested: requested_format,
                    closest: None,
                    fallback: fallback_for(fallback_policy, OutputFallback::SharedMixFormat),
                }
            })?;
            (parsed, mix.as_ptr())
        };

        let mut default_period_hns = 0_i64;
        let mut minimum_period_hns = 0_i64;
        unsafe {
            client.GetDevicePeriod(
                Some(&raw mut default_period_hns),
                Some(&raw mut minimum_period_hns),
            )
        }
        .map_err(|error| SharedOutputError::Backend {
            operation: RenderOperation::Initialize,
            hresult: Some(error.code().0),
            fallback: fallback_for(fallback_policy, OutputFallback::DefaultDevice),
        })?;
        let requested_buffer_hns = configured_period_frames
            .map_or(default_period_hns, |frames| {
                frames_to_hns(frames, negotiated_format.sample_rate_hz())
            })
            .max(minimum_period_hns);
        let flags = AUDCLNT_STREAMFLAGS_EVENTCALLBACK
            | AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM
            | AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY;
        unsafe {
            client.Initialize(
                AUDCLNT_SHAREMODE_SHARED,
                flags,
                requested_buffer_hns,
                0,
                format_ptr,
                None,
            )
        }
        .map_err(|error| SharedOutputError::Backend {
            operation: RenderOperation::Initialize,
            hresult: Some(error.code().0),
            fallback: fallback_for(fallback_policy, OutputFallback::SharedMixFormat),
        })?;
        let buffer_frames =
            unsafe { client.GetBufferSize() }.map_err(|error| SharedOutputError::Backend {
                operation: RenderOperation::Initialize,
                hresult: Some(error.code().0),
                fallback: fallback_for(fallback_policy, OutputFallback::DefaultDevice),
            })? as usize;
        let period_frames = configured_period_frames
            .unwrap_or_else(|| {
                usize::try_from(hns_to_frames(
                    default_period_hns,
                    negotiated_format.sample_rate_hz(),
                ))
                .unwrap_or(usize::MAX)
            })
            .max(1)
            .min(buffer_frames.max(1));
        let latency_hns = unsafe { client.GetStreamLatency() }.ok();
        let latency_frames =
            latency_hns.map(|hns| hns_to_frames(hns, negotiated_format.sample_rate_hz()));
        let event =
            unsafe { CreateEventW(None, false, false, PCWSTR::null()) }.map_err(|error| {
                SharedOutputError::Backend {
                    operation: RenderOperation::CreateEvent,
                    hresult: Some(error.code().0),
                    fallback: fallback_for(fallback_policy, OutputFallback::DefaultDevice),
                }
            })?;
        let event = EventHandle(event);
        unsafe { client.SetEventHandle(event.0) }.map_err(|error| {
            // The handle is not owned by the client when SetEventHandle
            // rejects it, so let the local RAII wrapper close it here.
            let _ = event;
            SharedOutputError::Backend {
                operation: RenderOperation::SetEventHandle,
                hresult: Some(error.code().0),
                fallback: fallback_for(fallback_policy, OutputFallback::DefaultDevice),
            }
        })?;
        let render_client =
            unsafe { client.GetService::<IAudioRenderClient>() }.map_err(|error| {
                SharedOutputError::Backend {
                    operation: RenderOperation::Initialize,
                    hresult: Some(error.code().0),
                    fallback: fallback_for(fallback_policy, OutputFallback::DefaultDevice),
                }
            })?;
        Ok(Self {
            _apartment: apartment,
            client,
            render_client,
            event,
            device,
            format: negotiated_format,
            buffer_frames,
            period_frames,
            latency_frames,
        })
    }
}

impl WasapiExclusiveBackend {
    #[allow(clippy::too_many_lines)]
    fn open(
        apartment: ComApartment,
        device: DeviceInfo,
        format: AudioFormat,
        configured_period_frames: Option<usize>,
        fallback_policy: FallbackPolicy,
    ) -> Result<Self, ExclusiveOutputError> {
        let client = activate_client(&device).map_err(|hresult| ExclusiveOutputError::Backend {
            operation: RenderOperation::Initialize,
            hresult: Some(hresult),
            fallback: exclusive_fallback_for(fallback_policy, OutputFallback::SharedMode),
        })?;
        let wave = wave_format(format).map_err(|error| match error {
            SharedOutputError::InvalidConfiguration { field } => {
                ExclusiveOutputError::InvalidConfiguration { field }
            }
            _ => ExclusiveOutputError::InvalidConfiguration {
                field: "audio format",
            },
        })?;
        let mut default_period_hns = 0_i64;
        let mut minimum_period_hns = 0_i64;
        unsafe {
            client.GetDevicePeriod(
                Some(&raw mut default_period_hns),
                Some(&raw mut minimum_period_hns),
            )
        }
        .map_err(|error| ExclusiveOutputError::Backend {
            operation: RenderOperation::Initialize,
            hresult: Some(error.code().0),
            fallback: exclusive_fallback_for(fallback_policy, OutputFallback::SharedMode),
        })?;
        let requested_period_hns = configured_period_frames
            .map_or(default_period_hns, |frames| {
                frames_to_hns(frames, format.sample_rate_hz())
            })
            .max(minimum_period_hns);
        // Exclusive event-driven streams use the same nonzero buffer duration
        // and periodicity; no Shared-mode AUTOCONVERTPCM flags are enabled.
        unsafe {
            client.Initialize(
                AUDCLNT_SHAREMODE_EXCLUSIVE,
                AUDCLNT_STREAMFLAGS_EVENTCALLBACK,
                requested_period_hns,
                requested_period_hns,
                &raw const wave,
                None,
            )
        }
        .map_err(|error| ExclusiveOutputError::Backend {
            operation: RenderOperation::Initialize,
            hresult: Some(error.code().0),
            fallback: exclusive_fallback_for(fallback_policy, OutputFallback::SharedMode),
        })?;
        let buffer_frames =
            unsafe { client.GetBufferSize() }.map_err(|error| ExclusiveOutputError::Backend {
                operation: RenderOperation::Initialize,
                hresult: Some(error.code().0),
                fallback: exclusive_fallback_for(fallback_policy, OutputFallback::SharedMode),
            })? as usize;
        let period_frames = configured_period_frames
            .unwrap_or_else(|| {
                usize::try_from(hns_to_frames(default_period_hns, format.sample_rate_hz()))
                    .unwrap_or(usize::MAX)
            })
            .max(1)
            .min(buffer_frames.max(1));
        let latency_frames = unsafe { client.GetStreamLatency() }
            .ok()
            .map(|hns| hns_to_frames(hns, format.sample_rate_hz()));
        let event =
            unsafe { CreateEventW(None, false, false, PCWSTR::null()) }.map_err(|error| {
                ExclusiveOutputError::Backend {
                    operation: RenderOperation::CreateEvent,
                    hresult: Some(error.code().0),
                    fallback: exclusive_fallback_for(fallback_policy, OutputFallback::SharedMode),
                }
            })?;
        let event = EventHandle(event);
        unsafe { client.SetEventHandle(event.0) }.map_err(|error| {
            let _ = event;
            ExclusiveOutputError::Backend {
                operation: RenderOperation::SetEventHandle,
                hresult: Some(error.code().0),
                fallback: exclusive_fallback_for(fallback_policy, OutputFallback::SharedMode),
            }
        })?;
        let render_client =
            unsafe { client.GetService::<IAudioRenderClient>() }.map_err(|error| {
                ExclusiveOutputError::Backend {
                    operation: RenderOperation::Initialize,
                    hresult: Some(error.code().0),
                    fallback: exclusive_fallback_for(fallback_policy, OutputFallback::SharedMode),
                }
            })?;
        Ok(Self {
            _apartment: apartment,
            client,
            render_client,
            event,
            device,
            format,
            buffer_frames,
            period_frames,
            latency_frames,
        })
    }
}

impl SharedRenderBackend for WasapiExclusiveBackend {
    fn device(&self) -> &DeviceInfo {
        &self.device
    }

    fn negotiated_format(&self) -> AudioFormat {
        self.format
    }

    fn buffer_size_frames(&self) -> usize {
        self.buffer_frames
    }

    fn period_frames(&self) -> usize {
        self.period_frames
    }

    fn estimated_latency_frames(&self) -> Option<u64> {
        self.latency_frames
    }

    fn output_observation(&self, request: OutputRequest) -> OutputObservation {
        let source = request.source_format();
        let conversion = if source.sample_format() == self.format.sample_format() {
            FormatConversionStatus::NotApplied
        } else {
            FormatConversionStatus::Applied
        };
        let resampling = if source.sample_rate_hz() == self.format.sample_rate_hz() {
            ResamplingStatus::NotApplied
        } else {
            ResamplingStatus::Applied
        };
        let mapping = if source.channels() == self.format.channels() {
            ChannelMappingStatus::Identity
        } else {
            ChannelMappingStatus::Remixed
        };
        OutputObservation::new(
            Some(self.format),
            Some(PlaybackRate::NORMAL),
            Some(Volume::UNITY),
            self.latency_frames,
            conversion,
            resampling,
            mapping,
            Some(false),
            Some(false),
        )
    }

    fn current_padding_frames(&mut self) -> Result<usize, RenderBackendError> {
        unsafe { self.client.GetCurrentPadding() }
            .map(|padding| padding as usize)
            .map_err(|error| backend_error(RenderOperation::ReadPadding, error.code().0))
    }

    fn wait_for_event(&mut self) -> Result<RenderEvent, RenderBackendError> {
        let result = unsafe { WaitForSingleObject(self.event.0, INFINITE) };
        if result == WAIT_OBJECT_0 {
            Ok(RenderEvent::Ready)
        } else if result == WAIT_FAILED {
            Err(RenderBackendError::Api {
                operation: RenderOperation::WaitForEvent,
                hresult: -1,
            })
        } else {
            Ok(RenderEvent::Timeout)
        }
    }

    fn start(&mut self) -> Result<(), RenderBackendError> {
        unsafe { self.client.Start() }
            .map_err(|error| backend_error(RenderOperation::Start, error.code().0))
    }

    fn stop(&mut self) -> Result<(), RenderBackendError> {
        unsafe { self.client.Stop() }
            .map_err(|error| backend_error(RenderOperation::Stop, error.code().0))
    }

    fn write_interleaved_f32(
        &mut self,
        frames: usize,
        samples: &[f32],
    ) -> Result<(), RenderBackendError> {
        let expected_samples = frames
            .checked_mul(usize::from(self.format.channels()))
            .ok_or(RenderBackendError::InvalidBuffer)?;
        if samples.len() != expected_samples || frames > self.buffer_frames {
            return Err(RenderBackendError::InvalidBuffer);
        }
        let frames_u32 = u32::try_from(frames).map_err(|_| RenderBackendError::InvalidBuffer)?;
        let data = unsafe { self.render_client.GetBuffer(frames_u32) }
            .map_err(|error| backend_error(RenderOperation::AcquireBuffer, error.code().0))?;
        unsafe { write_samples(data, self.format.sample_format(), samples) };
        unsafe { self.render_client.ReleaseBuffer(frames_u32, 0) }
            .map_err(|error| backend_error(RenderOperation::ReleaseBuffer, error.code().0))
    }
}

impl SharedRenderBackend for WasapiSharedBackend {
    fn device(&self) -> &DeviceInfo {
        &self.device
    }

    fn negotiated_format(&self) -> AudioFormat {
        self.format
    }

    fn buffer_size_frames(&self) -> usize {
        self.buffer_frames
    }

    fn period_frames(&self) -> usize {
        self.period_frames
    }

    fn estimated_latency_frames(&self) -> Option<u64> {
        self.latency_frames
    }

    fn current_padding_frames(&mut self) -> Result<usize, RenderBackendError> {
        unsafe { self.client.GetCurrentPadding() }
            .map(|padding| padding as usize)
            .map_err(|error| backend_error(RenderOperation::ReadPadding, error.code().0))
    }

    fn wait_for_event(&mut self) -> Result<RenderEvent, RenderBackendError> {
        let result = unsafe { WaitForSingleObject(self.event.0, INFINITE) };
        if result == WAIT_OBJECT_0 {
            Ok(RenderEvent::Ready)
        } else if result == WAIT_FAILED {
            Err(RenderBackendError::Api {
                operation: RenderOperation::WaitForEvent,
                hresult: -1,
            })
        } else {
            Ok(RenderEvent::Timeout)
        }
    }

    fn start(&mut self) -> Result<(), RenderBackendError> {
        unsafe { self.client.Start() }
            .map_err(|error| backend_error(RenderOperation::Start, error.code().0))
    }

    fn stop(&mut self) -> Result<(), RenderBackendError> {
        unsafe { self.client.Stop() }
            .map_err(|error| backend_error(RenderOperation::Stop, error.code().0))
    }

    fn write_interleaved_f32(
        &mut self,
        frames: usize,
        samples: &[f32],
    ) -> Result<(), RenderBackendError> {
        let expected_samples = frames
            .checked_mul(usize::from(self.format.channels()))
            .ok_or(RenderBackendError::InvalidBuffer)?;
        if samples.len() != expected_samples || frames > self.buffer_frames {
            return Err(RenderBackendError::InvalidBuffer);
        }
        let frames_u32 = u32::try_from(frames).map_err(|_| RenderBackendError::InvalidBuffer)?;
        let data = unsafe { self.render_client.GetBuffer(frames_u32) }
            .map_err(|error| backend_error(RenderOperation::AcquireBuffer, error.code().0))?;
        unsafe { write_samples(data, self.format.sample_format(), samples) };
        unsafe { self.render_client.ReleaseBuffer(frames_u32, 0) }
            .map_err(|error| backend_error(RenderOperation::ReleaseBuffer, error.code().0))
    }
}

fn backend_error(operation: RenderOperation, hresult: i32) -> RenderBackendError {
    if hresult == AUDCLNT_E_DEVICE_INVALIDATED.0 {
        RenderBackendError::DeviceRemoved
    } else {
        RenderBackendError::Api { operation, hresult }
    }
}

/// The event handle is created once during setup and closed after the client
/// has stopped. It is never touched by Core or UI code.
#[derive(Debug)]
struct EventHandle(windows::Win32::Foundation::HANDLE);

impl Drop for EventHandle {
    fn drop(&mut self) {
        if !self.0.is_invalid() {
            let _ = unsafe { CloseHandle(self.0) };
        }
    }
}

fn wave_format(format: AudioFormat) -> Result<WAVEFORMATEX, SharedOutputError> {
    let channels = u16::from(format.channels());
    let bits = u16::from(format.sample_format().bits());
    let bytes_per_sample = u32::from(bits.div_ceil(8));
    let block_align = u32::from(channels)
        .checked_mul(bytes_per_sample)
        .and_then(|value| u16::try_from(value).ok())
        .ok_or(SharedOutputError::InvalidConfiguration {
            field: "audio format block alignment",
        })?;
    let average_bytes = format
        .sample_rate_hz()
        .checked_mul(u32::from(block_align))
        .ok_or(SharedOutputError::InvalidConfiguration {
            field: "audio format byte rate",
        })?;
    Ok(WAVEFORMATEX {
        wFormatTag: match format.sample_format() {
            // These Windows constants are defined as u32 even though the
            // WAVEFORMATEX tag is specified as a 16-bit value.
            SampleFormat::F32 => u16::try_from(WAVE_FORMAT_IEEE_FLOAT).map_err(|_| {
                SharedOutputError::InvalidConfiguration {
                    field: "IEEE float wave format tag",
                }
            })?,
            SampleFormat::I16 | SampleFormat::I24 | SampleFormat::I32 => {
                u16::try_from(WAVE_FORMAT_PCM).map_err(|_| {
                    SharedOutputError::InvalidConfiguration {
                        field: "PCM wave format tag",
                    }
                })?
            }
        },
        nChannels: channels,
        nSamplesPerSec: format.sample_rate_hz(),
        nAvgBytesPerSec: average_bytes,
        nBlockAlign: block_align,
        wBitsPerSample: bits,
        cbSize: 0,
    })
}

fn parse_wave_format(pointer: *const WAVEFORMATEX) -> Result<AudioFormat, FormatError> {
    if pointer.is_null() {
        return Err(FormatError::InvalidSampleRate { actual: 0 });
    }
    let base = unsafe { pointer.read_unaligned() };
    let float_tag = u16::try_from(WAVE_FORMAT_IEEE_FLOAT).unwrap_or(u16::MAX);
    let pcm_tag = u16::try_from(WAVE_FORMAT_PCM).unwrap_or(u16::MAX);
    let extensible_tag = u16::try_from(WAVE_FORMAT_EXTENSIBLE).unwrap_or(u16::MAX);
    let sample_format = if base.wFormatTag == float_tag {
        SampleFormat::F32
    } else if base.wFormatTag == pcm_tag {
        SampleFormat::from_source_bits(source_bits(base.wBitsPerSample))?
    } else if base.wFormatTag == extensible_tag && base.cbSize >= 22 {
        let extended = unsafe { (pointer.cast::<WAVEFORMATEXTENSIBLE>()).read_unaligned() };
        let sub_format = unsafe { ptr::addr_of!(extended.SubFormat).read_unaligned() };
        if sub_format == IEEE_FLOAT_SUBTYPE {
            SampleFormat::F32
        } else if sub_format == PCM_SUBTYPE {
            SampleFormat::from_source_bits(source_bits(base.wBitsPerSample))?
        } else {
            return Err(FormatError::UnsupportedBitDepth {
                actual: source_bits(base.wBitsPerSample),
            });
        }
    } else {
        return Err(FormatError::UnsupportedBitDepth {
            actual: source_bits(base.wBitsPerSample),
        });
    };
    AudioFormat::new(
        base.nSamplesPerSec,
        u8::try_from(base.nChannels).unwrap_or(u8::MAX),
        sample_format,
    )
}

fn source_bits(bits: u16) -> u8 {
    u8::try_from(bits.min(u16::from(u8::MAX))).unwrap_or(u8::MAX)
}

fn frames_to_hns(frames: usize, sample_rate: u32) -> i64 {
    if sample_rate == 0 {
        return 0;
    }
    let numerator = (frames as u128).saturating_mul(10_000_000);
    let value = numerator.div_ceil(u128::from(sample_rate));
    i64::try_from(value).unwrap_or(i64::MAX)
}

fn hns_to_frames(hns: i64, sample_rate: u32) -> u64 {
    if hns <= 0 || sample_rate == 0 {
        return 0;
    }
    let numerator = u128::try_from(hns)
        .unwrap_or(0)
        .saturating_mul(u128::from(sample_rate));
    u64::try_from(numerator.div_ceil(10_000_000)).unwrap_or(u64::MAX)
}

unsafe fn write_samples(data: *mut u8, format: SampleFormat, samples: &[f32]) {
    let bytes_per_sample = usize::from(format.bits().div_ceil(8));
    let destination =
        unsafe { std::slice::from_raw_parts_mut(data, samples.len() * bytes_per_sample) };
    match format {
        SampleFormat::F32 => {
            for (destination, source) in destination.chunks_exact_mut(4).zip(samples) {
                destination.copy_from_slice(&source.to_le_bytes());
            }
        }
        SampleFormat::I16 => {
            for (destination, source) in destination.chunks_exact_mut(2).zip(samples) {
                destination.copy_from_slice(&to_i16(*source).to_le_bytes());
            }
        }
        SampleFormat::I24 => {
            for (index, source) in samples.iter().copied().enumerate() {
                let value = to_i24(source).to_le_bytes();
                let offset = index * 3;
                destination[offset..offset + 3].copy_from_slice(&value[..3]);
            }
        }
        SampleFormat::I32 => {
            for (destination, source) in destination.chunks_exact_mut(4).zip(samples) {
                destination.copy_from_slice(&to_i32(*source).to_le_bytes());
            }
        }
    }
}

fn normalized_sample(value: f32) -> f64 {
    if value.is_finite() {
        f64::from(value).clamp(-1.0, 1.0)
    } else {
        0.0
    }
}

#[allow(clippy::cast_possible_truncation)]
fn to_i16(value: f32) -> i16 {
    let scaled = normalized_sample(value) * 32_768.0;
    scaled.round().clamp(-32_768.0, 32_767.0) as i16
}

#[allow(clippy::cast_possible_truncation)]
fn to_i24(value: f32) -> i32 {
    let scaled = normalized_sample(value) * 8_388_608.0;
    scaled.round().clamp(-8_388_608.0, 8_388_607.0) as i32
}

#[allow(clippy::cast_possible_truncation)]
fn to_i32(value: f32) -> i32 {
    let scaled = normalized_sample(value) * 2_147_483_648.0;
    scaled.round().clamp(-2_147_483_648.0, 2_147_483_647.0) as i64 as i32
}

#[cfg(test)]
mod tests {
    use std::ptr;

    use super::{
        frames_to_hns, hns_to_frames, parse_wave_format, to_i16, to_i24, to_i32, wave_format,
        write_samples,
    };
    use aurorix_audio::format::{AudioFormat, SampleFormat};
    use windows::Win32::Media::{
        KernelStreaming::WAVE_FORMAT_EXTENSIBLE, Multimedia::WAVE_FORMAT_IEEE_FLOAT,
    };

    #[test]
    fn hns_frame_conversion_rounds_up_for_latency_reporting() {
        assert_eq!(frames_to_hns(480, 48_000), 100_000);
        assert_eq!(hns_to_frames(100_000, 48_000), 480);
        assert_eq!(hns_to_frames(1, 48_000), 1);
    }

    #[test]
    fn supported_wave_formats_round_trip() {
        for sample_format in [
            SampleFormat::I16,
            SampleFormat::I24,
            SampleFormat::I32,
            SampleFormat::F32,
        ] {
            let format = AudioFormat::new(96_000, 2, sample_format).expect("format is valid");
            let wave = wave_format(format).expect("wave format is valid");
            assert_eq!(parse_wave_format(std::ptr::addr_of!(wave)), Ok(format));
        }
    }

    #[test]
    fn integer_conversion_is_bounded_and_deterministic() {
        assert_eq!(to_i16(-1.0), i16::MIN);
        assert_eq!(to_i16(1.0), i16::MAX);
        assert_eq!(to_i24(-1.0), -8_388_608);
        assert_eq!(to_i24(1.0), 8_388_607);
        assert_eq!(to_i32(-1.0), i32::MIN);
        assert_eq!(to_i32(1.0), i32::MAX);
        assert_eq!(to_i16(f32::NAN), 0);
    }

    #[test]
    fn render_buffer_writes_little_endian_samples_without_alignment_assumptions() {
        let mut bytes = [0_u8; 8];
        unsafe { write_samples(bytes.as_mut_ptr(), SampleFormat::I16, &[1.0, -1.0]) };
        assert_eq!(bytes, [0xff, 0x7f, 0x00, 0x80, 0, 0, 0, 0]);
    }

    #[test]
    fn float_wave_tag_is_the_internal_render_format() {
        let format = AudioFormat::f32(48_000, 2).expect("format is valid");
        let wave = wave_format(format).expect("wave format is valid");
        let tag = unsafe { ptr::addr_of!(wave.wFormatTag).read_unaligned() };
        assert_eq!(
            tag,
            u16::try_from(WAVE_FORMAT_IEEE_FLOAT).expect("float tag fits")
        );
        assert_ne!(
            tag,
            u16::try_from(WAVE_FORMAT_EXTENSIBLE).expect("extensible tag fits")
        );
    }
}
