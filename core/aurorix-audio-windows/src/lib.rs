//! Windows x64 WASAPI Shared and Exclusive output boundary.
//!
//! The control plane owns device selection, format negotiation, and report
//! construction. The render plane owns only a preallocated scratch buffer and
//! calls a caller-provided realtime renderer. No Core database, FFI, UI, or
//! provider state crosses this crate's render method.

#![cfg_attr(
    all(windows, not(target_arch = "x86_64")),
    doc = "Windows x86 and ARM64 are not supported."
)]

use std::{error::Error, fmt};

use aurorix_audio::{
    format::AudioFormat,
    output_report::{
        ChannelMappingStatus, FormatConversionStatus, OutputObservation, OutputReport,
        OutputRequest, PlaybackRate, ResamplingStatus, Volume,
    },
    realtime::{RealtimeConsumer, RenderOutcome},
};

#[cfg(all(windows, not(target_arch = "x86_64")))]
compile_error!("Aurorix Windows audio supports x64 only");

/// The event-driven shared-mode callback period target from the audio
/// execution contract.
pub const DEFAULT_PERIOD_MILLISECONDS: u32 = 10;

/// A device-local endpoint identity. It is not a filesystem path and must not
/// be copied into Sync or portable playback intent.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct DeviceId(String);

impl DeviceId {
    /// Creates a device identity, rejecting only an empty identifier.
    ///
    /// # Errors
    ///
    /// Returns [`DeviceIdError::Empty`] when `value` is empty.
    pub fn new(value: impl Into<String>) -> Result<Self, DeviceIdError> {
        let value = value.into();
        if value.is_empty() {
            return Err(DeviceIdError::Empty);
        }
        Ok(Self(value))
    }

    /// Returns the opaque device identity.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Invalid device identity input.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeviceIdError {
    /// The endpoint identity was empty.
    Empty,
}

impl fmt::Display for DeviceIdError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("audio device id must not be empty")
    }
}

impl Error for DeviceIdError {}

/// A coarse endpoint state safe to expose to the Core/UI control plane.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DeviceState {
    /// The endpoint can currently be activated.
    Active,
    /// The endpoint exists but is disabled.
    Disabled,
    /// The endpoint is not currently present.
    NotPresent,
    /// The endpoint is known but unplugged.
    Unplugged,
}

/// A bounded device projection. Friendly-name lookup is deliberately an
/// adapter concern; the platform backend may provide a safe display label.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceInfo {
    pub(crate) id: DeviceId,
    pub(crate) name: String,
    pub(crate) state: DeviceState,
    pub(crate) is_default: bool,
}

impl DeviceInfo {
    /// Creates a device projection for an adapter or deterministic fake.
    ///
    /// # Errors
    ///
    /// Returns [`DeviceIdError::Empty`] when `id` is empty.
    pub fn new(
        id: impl Into<String>,
        name: impl Into<String>,
        state: DeviceState,
        is_default: bool,
    ) -> Result<Self, DeviceIdError> {
        Ok(Self {
            id: DeviceId::new(id)?,
            name: name.into(),
            state,
            is_default,
        })
    }

    /// Returns the opaque endpoint identity.
    #[must_use]
    pub fn id(&self) -> &DeviceId {
        &self.id
    }

    /// Returns the safe display name.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Returns the current endpoint state.
    #[must_use]
    pub const fn state(&self) -> DeviceState {
        self.state
    }

    /// Returns whether this endpoint was the default render endpoint when
    /// enumerated.
    #[must_use]
    pub const fn is_default(&self) -> bool {
        self.is_default
    }
}

/// How a caller selects the output endpoint.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum DeviceSelector {
    /// Resolve the current Windows default render endpoint.
    #[default]
    Default,
    /// Resolve one previously enumerated endpoint identity.
    Id(DeviceId),
}

/// A control-plane fallback that can be surfaced to policy/UI code.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum OutputFallback {
    /// Retry on the current default render endpoint.
    DefaultDevice,
    /// Retry negotiation with the endpoint's shared mix format.
    SharedMixFormat,
    /// Retry the same endpoint in WASAPI Shared mode.
    SharedMode,
}

/// Whether a failed operation should advertise a fallback.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum FallbackPolicy {
    /// Do not offer an automatic fallback.
    Disabled,
    /// Offer a safe backend retry where applicable.
    #[default]
    DefaultDevice,
}

/// Shared output configuration. All fields are control-plane state and are
/// frozen before the event-driven renderer starts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SharedOutputConfig {
    request: OutputRequest,
    selector: DeviceSelector,
    fallback_policy: FallbackPolicy,
    period_frames: Option<usize>,
}

impl SharedOutputConfig {
    /// Creates Shared output configuration from the existing output request.
    #[must_use]
    pub const fn new(request: OutputRequest) -> Self {
        Self {
            request,
            selector: DeviceSelector::Default,
            fallback_policy: FallbackPolicy::DefaultDevice,
            period_frames: None,
        }
    }

    /// Selects one endpoint by its opaque identity.
    #[must_use]
    pub fn with_device(mut self, selector: DeviceSelector) -> Self {
        self.selector = selector;
        self
    }

    /// Sets the fallback policy.
    #[must_use]
    pub const fn with_fallback_policy(mut self, policy: FallbackPolicy) -> Self {
        self.fallback_policy = policy;
        self
    }

    /// Overrides the backend's preferred event period in frames.
    #[must_use]
    pub const fn with_period_frames(mut self, frames: Option<usize>) -> Self {
        self.period_frames = frames;
        self
    }

    /// Returns the output request.
    #[must_use]
    pub const fn request(&self) -> OutputRequest {
        self.request
    }

    /// Returns the device selector.
    #[must_use]
    pub const fn selector(&self) -> &DeviceSelector {
        &self.selector
    }

    /// Returns the fallback policy.
    #[must_use]
    pub const fn fallback_policy(&self) -> FallbackPolicy {
        self.fallback_policy
    }

    /// Returns an explicitly configured period, if any.
    #[must_use]
    pub const fn period_frames(&self) -> Option<usize> {
        self.period_frames
    }
}

/// Exclusive-mode output configuration. The request and endpoint selection
/// are shared with the existing Shared boundary; only the WASAPI stream mode
/// differs. All fields are frozen before the event-driven renderer starts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExclusiveOutputConfig {
    request: OutputRequest,
    selector: DeviceSelector,
    fallback_policy: FallbackPolicy,
    period_frames: Option<usize>,
}

impl ExclusiveOutputConfig {
    /// Creates Exclusive output configuration from the existing output request.
    #[must_use]
    pub const fn new(request: OutputRequest) -> Self {
        Self {
            request,
            selector: DeviceSelector::Default,
            fallback_policy: FallbackPolicy::DefaultDevice,
            period_frames: None,
        }
    }

    /// Selects one endpoint by its opaque identity.
    #[must_use]
    pub fn with_device(mut self, selector: DeviceSelector) -> Self {
        self.selector = selector;
        self
    }

    /// Sets the fallback policy.
    #[must_use]
    pub const fn with_fallback_policy(mut self, policy: FallbackPolicy) -> Self {
        self.fallback_policy = policy;
        self
    }

    /// Overrides the backend's preferred event period in frames.
    #[must_use]
    pub const fn with_period_frames(mut self, frames: Option<usize>) -> Self {
        self.period_frames = frames;
        self
    }

    /// Returns the output request.
    #[must_use]
    pub const fn request(&self) -> OutputRequest {
        self.request
    }

    /// Returns the device selector.
    #[must_use]
    pub const fn selector(&self) -> &DeviceSelector {
        &self.selector
    }

    /// Returns the fallback policy.
    #[must_use]
    pub const fn fallback_policy(&self) -> FallbackPolicy {
        self.fallback_policy
    }

    /// Returns an explicitly configured period, if any.
    #[must_use]
    pub const fn period_frames(&self) -> Option<usize> {
        self.period_frames
    }

    /// Builds the same-endpoint Shared configuration for an explicit retry.
    #[must_use]
    pub fn shared_config(&self) -> SharedOutputConfig {
        SharedOutputConfig {
            request: self.request,
            selector: self.selector.clone(),
            fallback_policy: self.fallback_policy,
            period_frames: self.period_frames,
        }
    }
}

/// A typed failure from Exclusive setup. A Shared retry is only advertised
/// when the configured fallback policy permits it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExclusiveOutputError {
    /// A control-plane configuration value was invalid.
    InvalidConfiguration { field: &'static str },
    /// No endpoint can currently satisfy the operation.
    Unavailable {
        reason: OutputUnavailableReason,
        fallback: Option<OutputFallback>,
    },
    /// The endpoint rejected the requested Exclusive format.
    FormatNegotiation {
        requested: AudioFormat,
        closest: Option<AudioFormat>,
        fallback: Option<OutputFallback>,
    },
    /// Exclusive initialization or runtime setup failed.
    Backend {
        operation: RenderOperation,
        hresult: Option<i32>,
        fallback: Option<OutputFallback>,
    },
}

impl fmt::Display for ExclusiveOutputError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidConfiguration { field } => {
                write!(formatter, "invalid Exclusive output configuration: {field}")
            }
            Self::Unavailable { reason, fallback } => {
                write!(
                    formatter,
                    "Exclusive output unavailable: {reason:?} ({fallback:?})"
                )
            }
            Self::FormatNegotiation {
                requested,
                closest,
                fallback,
            } => write!(
                formatter,
                "Exclusive format negotiation rejected {requested:?}, closest={closest:?}, fallback={fallback:?}"
            ),
            Self::Backend {
                operation,
                hresult,
                fallback,
            } => write!(
                formatter,
                "Exclusive backend {operation} failed ({hresult:?}), fallback={fallback:?}"
            ),
        }
    }
}

impl Error for ExclusiveOutputError {}

impl ExclusiveOutputError {
    /// Returns the policy-provided retry, if one is safe to offer.
    #[must_use]
    pub const fn fallback(&self) -> Option<OutputFallback> {
        match self {
            Self::InvalidConfiguration { .. } => None,
            Self::Unavailable { fallback, .. }
            | Self::FormatNegotiation { fallback, .. }
            | Self::Backend { fallback, .. } => *fallback,
        }
    }
}

/// Result of asking an endpoint whether an Exclusive format is usable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExclusiveFormatSupport {
    /// The requested format is accepted exactly.
    Exact,
    /// The endpoint supplied a usable closest format.
    ///
    /// This is available to abstract backends. Native WASAPI Exclusive
    /// negotiation accepts exact support only; Shared negotiation owns its
    /// own closest/mix-format policy.
    Closest(AudioFormat),
    /// The endpoint cannot accept the requested format in Exclusive mode.
    Unsupported,
}

/// Resolves the actual format for an Exclusive stream before backend creation.
///
/// A closest format is a successful negotiation, but the resulting output
/// report will expose it separately from the requested format. Unsupported
/// formats produce a typed Shared-mode retry only when policy allows it.
///
/// # Errors
///
/// Returns [`ExclusiveOutputError::FormatNegotiation`] when the endpoint does
/// not accept the requested format.
pub fn negotiate_exclusive_format(
    request: OutputRequest,
    support: ExclusiveFormatSupport,
    fallback_policy: FallbackPolicy,
) -> Result<AudioFormat, ExclusiveOutputError> {
    let requested = request
        .requested_output_format()
        .unwrap_or(request.source_format());
    match support {
        ExclusiveFormatSupport::Exact => Ok(requested),
        ExclusiveFormatSupport::Closest(format) => Ok(format),
        ExclusiveFormatSupport::Unsupported => Err(ExclusiveOutputError::FormatNegotiation {
            requested,
            closest: None,
            fallback: fallback_for(fallback_policy, OutputFallback::SharedMode),
        }),
    }
}

/// The output selected after an Exclusive attempt and optional Shared retry.
#[derive(Debug)]
pub enum ExclusiveOpenOutput<E: SharedRenderBackend, S: SharedRenderBackend> {
    /// Exclusive mode was prepared successfully.
    Exclusive(SharedOutput<E>),
    /// Exclusive failed and the same request was prepared in Shared mode.
    SharedFallback(SharedOutput<S>),
}

/// A failed Exclusive attempt and, when applicable, its failed Shared retry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExclusiveOpenError {
    /// Exclusive failed without an allowed Shared retry.
    Exclusive(ExclusiveOutputError),
    /// Exclusive advertised Shared fallback, but the Shared attempt failed.
    SharedFallback {
        /// The original Exclusive failure.
        exclusive: ExclusiveOutputError,
        /// The typed Shared retry failure.
        shared: SharedOutputError,
    },
}

impl fmt::Display for ExclusiveOpenError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Exclusive(error) => write!(formatter, "Exclusive output failed: {error}"),
            Self::SharedFallback { exclusive, shared } => write!(
                formatter,
                "Exclusive output failed ({exclusive}); Shared fallback failed ({shared})"
            ),
        }
    }
}

impl Error for ExclusiveOpenError {}

/// Control-plane factory used by the Windows adapter and deterministic fakes.
///
/// Every method runs before the event-driven render loop. Implementations must
/// return a fully prepared [`SharedOutput`], so its render boundary remains
/// the same for both stream modes.
pub trait ExclusiveBackendFactory {
    /// Fully negotiated Exclusive backend type.
    type ExclusiveBackend: SharedRenderBackend;
    /// Fully negotiated Shared backend type.
    type SharedBackend: SharedRenderBackend;

    /// Checks whether the requested format is exact, closest, or unsupported.
    ///
    /// # Errors
    ///
    /// Returns [`ExclusiveOutputError`] when the endpoint cannot be queried
    /// or the requested format cannot be represented.
    fn exclusive_format_support(
        &mut self,
        requested: AudioFormat,
    ) -> Result<ExclusiveFormatSupport, ExclusiveOutputError>;

    /// Opens a fully negotiated Exclusive backend.
    ///
    /// # Errors
    ///
    /// Returns [`ExclusiveOutputError`] when backend setup or report
    /// preparation fails.
    fn open_exclusive(
        &mut self,
        config: &ExclusiveOutputConfig,
        negotiated_format: AudioFormat,
    ) -> Result<SharedOutput<Self::ExclusiveBackend>, ExclusiveOutputError>;

    /// Opens a fully negotiated Shared backend for an explicit fallback.
    ///
    /// # Errors
    ///
    /// Returns [`SharedOutputError`] when Shared fallback setup fails.
    fn open_shared(
        &mut self,
        config: &SharedOutputConfig,
    ) -> Result<SharedOutput<Self::SharedBackend>, SharedOutputError>;
}

/// Opens Exclusive mode and performs the explicitly typed Shared fallback.
///
/// The fallback is attempted only for an error carrying
/// [`OutputFallback::SharedMode`]. Disabled fallback policy therefore returns
/// the original Exclusive error without invoking the Shared factory method.
/// All work here is control-plane setup; the returned output uses the existing
/// allocation-free [`SharedOutput::render_event`] boundary.
///
/// # Errors
///
/// Returns [`ExclusiveOpenError::Exclusive`] when Exclusive fails without an
/// allowed retry, or [`ExclusiveOpenError::SharedFallback`] when the retry
/// itself fails.
pub fn open_exclusive_with_shared_fallback<F: ExclusiveBackendFactory>(
    factory: &mut F,
    config: &ExclusiveOutputConfig,
) -> Result<ExclusiveOpenOutput<F::ExclusiveBackend, F::SharedBackend>, ExclusiveOpenError> {
    let requested = config
        .request()
        .requested_output_format()
        .unwrap_or(config.request().source_format());
    let support = match factory.exclusive_format_support(requested) {
        Ok(support) => support,
        Err(error) => return open_shared_after_exclusive_failure(factory, config, error),
    };
    let negotiated =
        match negotiate_exclusive_format(config.request(), support, config.fallback_policy()) {
            Ok(format) => format,
            Err(error) => return open_shared_after_exclusive_failure(factory, config, error),
        };
    match factory.open_exclusive(config, negotiated) {
        Ok(output) => Ok(ExclusiveOpenOutput::Exclusive(output)),
        Err(error) => open_shared_after_exclusive_failure(factory, config, error),
    }
}

fn open_shared_after_exclusive_failure<F: ExclusiveBackendFactory>(
    factory: &mut F,
    config: &ExclusiveOutputConfig,
    exclusive: ExclusiveOutputError,
) -> Result<ExclusiveOpenOutput<F::ExclusiveBackend, F::SharedBackend>, ExclusiveOpenError> {
    if exclusive.fallback() != Some(OutputFallback::SharedMode) {
        return Err(ExclusiveOpenError::Exclusive(exclusive));
    }
    match factory.open_shared(&config.shared_config()) {
        Ok(output) => Ok(ExclusiveOpenOutput::SharedFallback(output)),
        Err(shared) => Err(ExclusiveOpenError::SharedFallback { exclusive, shared }),
    }
}

/// A device-enumeration failure that does not expose paths or COM pointers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeviceEnumerationError {
    /// Windows audio services are unavailable.
    BackendUnavailable { hresult: Option<i32> },
    /// A COM operation failed.
    Api {
        operation: DeviceOperation,
        hresult: i32,
    },
    /// Windows returned malformed device identity data.
    InvalidDeviceId,
}

impl fmt::Display for DeviceEnumerationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BackendUnavailable { hresult } => {
                write!(formatter, "Windows audio backend unavailable ({hresult:?})")
            }
            Self::Api { operation, hresult } => {
                write!(formatter, "audio device {operation} failed ({hresult:#x})")
            }
            Self::InvalidDeviceId => formatter.write_str("Windows returned an invalid device id"),
        }
    }
}

impl Error for DeviceEnumerationError {}

/// Control-plane device discovery abstraction implemented by the Windows
/// `MMDevice` backend and available for deterministic host-side fakes.
pub trait DeviceEnumerator {
    /// Enumerates render endpoints.
    ///
    /// # Errors
    ///
    /// Returns a typed platform error when endpoint discovery fails.
    fn enumerate(&mut self) -> Result<Vec<DeviceInfo>, DeviceEnumerationError>;

    /// Resolves the current default render endpoint.
    ///
    /// # Errors
    ///
    /// Returns a typed platform error when default endpoint resolution fails.
    fn default_device(&mut self) -> Result<DeviceInfo, DeviceEnumerationError>;

    /// Resolves one endpoint by opaque identity.
    ///
    /// # Errors
    ///
    /// Returns a typed platform error when the endpoint cannot be resolved.
    fn device(&mut self, id: &DeviceId) -> Result<DeviceInfo, DeviceEnumerationError>;
}

/// Control-plane device operations used in typed diagnostics.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DeviceOperation {
    /// Initialize the COM apartment.
    InitializeCom,
    /// Create the `MMDevice` enumerator.
    CreateEnumerator,
    /// Enumerate active and inactive render endpoints.
    Enumerate,
    /// Resolve the default render endpoint.
    ResolveDefault,
    /// Resolve a selected endpoint identity.
    ResolveSelected,
    /// Read an endpoint identity.
    ReadId,
    /// Read an endpoint state.
    ReadState,
}

impl fmt::Display for DeviceOperation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            Self::InitializeCom => "COM initialization",
            Self::CreateEnumerator => "enumerator creation",
            Self::Enumerate => "enumeration",
            Self::ResolveDefault => "default resolution",
            Self::ResolveSelected => "selected resolution",
            Self::ReadId => "identity read",
            Self::ReadState => "state read",
        };
        formatter.write_str(name)
    }
}

/// Why a Shared output cannot currently be used.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum OutputUnavailableReason {
    /// No render endpoint was available.
    NoOutputDevice,
    /// The selected endpoint was removed or invalidated.
    DeviceRemoved,
    /// The selected endpoint is disabled.
    DeviceDisabled,
    /// The selected endpoint is unplugged.
    DeviceUnplugged,
    /// Windows audio services or the endpoint activation path is unavailable.
    BackendUnavailable,
}

/// The operation that failed on a render backend.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RenderOperation {
    /// Wait for the event-driven period signal.
    WaitForEvent,
    /// Read current endpoint padding.
    ReadPadding,
    /// Start the audio client.
    Start,
    /// Stop the audio client.
    Stop,
    /// Acquire the endpoint render buffer.
    AcquireBuffer,
    /// Release the endpoint render buffer.
    ReleaseBuffer,
    /// Set the event handle.
    SetEventHandle,
    /// Create the event handle.
    CreateEvent,
    /// Initialize the endpoint client.
    Initialize,
    /// Negotiate the endpoint format.
    NegotiateFormat,
}

impl fmt::Display for RenderOperation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            Self::WaitForEvent => "wait for event",
            Self::ReadPadding => "read padding",
            Self::Start => "start",
            Self::Stop => "stop",
            Self::AcquireBuffer => "acquire render buffer",
            Self::ReleaseBuffer => "release render buffer",
            Self::SetEventHandle => "set event handle",
            Self::CreateEvent => "create event",
            Self::Initialize => "initialize",
            Self::NegotiateFormat => "negotiate format",
        };
        formatter.write_str(name)
    }
}

/// A backend error that is safe to carry from the event-driven renderer to a
/// non-realtime coordinator.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RenderBackendError {
    /// The endpoint has disappeared or was invalidated.
    DeviceRemoved,
    /// The endpoint is not usable in its current state.
    Unavailable,
    /// The underlying platform API returned an HRESULT.
    Api {
        /// Operation that returned the HRESULT.
        operation: RenderOperation,
        /// Numeric HRESULT, without a platform error string allocation.
        hresult: i32,
    },
    /// The adapter's preallocated buffer contract was violated.
    InvalidBuffer,
}

/// A typed Shared open, render, or lifecycle failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SharedOutputError {
    /// A control-plane configuration value was invalid.
    InvalidConfiguration { field: &'static str },
    /// No endpoint can currently satisfy the operation.
    Unavailable {
        /// Stable reason for the unavailable state.
        reason: OutputUnavailableReason,
        /// Policy-provided retry, if one is safe to offer.
        fallback: Option<OutputFallback>,
    },
    /// The endpoint did not accept the requested format.
    FormatNegotiation {
        /// Format supplied by Core policy.
        requested: AudioFormat,
        /// Closest endpoint format, when Windows supplied one.
        closest: Option<AudioFormat>,
        /// Policy-provided retry, if one is safe to offer.
        fallback: Option<OutputFallback>,
    },
    /// A runtime backend operation failed.
    Backend {
        /// Operation that failed.
        operation: RenderOperation,
        /// Numeric HRESULT where available.
        hresult: Option<i32>,
        /// Policy-provided retry, if one is safe to offer.
        fallback: Option<OutputFallback>,
    },
}

impl fmt::Display for SharedOutputError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidConfiguration { field } => {
                write!(formatter, "invalid Shared output configuration: {field}")
            }
            Self::Unavailable { reason, fallback } => {
                write!(
                    formatter,
                    "Shared output unavailable: {reason:?} ({fallback:?})"
                )
            }
            Self::FormatNegotiation {
                requested,
                closest,
                fallback,
            } => write!(
                formatter,
                "Shared format negotiation rejected {requested:?}, closest={closest:?}, fallback={fallback:?}"
            ),
            Self::Backend {
                operation,
                hresult,
                fallback,
            } => write!(
                formatter,
                "Shared backend {operation} failed ({hresult:?}), fallback={fallback:?}"
            ),
        }
    }
}

impl Error for SharedOutputError {}

/// A notification produced by the event-driven scheduler.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RenderEvent {
    /// The endpoint has space for one render period.
    Ready,
    /// The scheduler observed no data-ready signal in a bounded wait.
    Timeout,
}

/// A result from one bounded render period.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RenderCycle {
    /// Frames requested from the endpoint for this period.
    pub requested_frames: usize,
    /// Result reported by the Core realtime renderer.
    pub outcome: RenderOutcome,
}

/// The only callback contract accepted by [`SharedOutput::render_event`].
/// Implementations must render into the supplied slice without blocking,
/// allocating, doing I/O, or calling FFI/UI code.
pub trait RealtimePcmRenderer {
    /// Fills the supplied interleaved output samples.
    fn render(&mut self, output: &mut [f32]) -> RenderOutcome;
}

impl RealtimePcmRenderer for RealtimeConsumer {
    fn render(&mut self, output: &mut [f32]) -> RenderOutcome {
        RealtimeConsumer::render(self, output)
    }
}

/// Backend operations needed by the Shared output control and render planes.
///
/// Implementations must allocate and initialize all resources before the
/// first call to `write_interleaved_f32`. That method is called on the
/// event-driven render thread and must not block or allocate in the adapter.
pub trait SharedRenderBackend {
    /// Returns the device projection owned by this backend.
    fn device(&self) -> &DeviceInfo;

    /// Returns the format accepted by the endpoint render buffer.
    fn negotiated_format(&self) -> AudioFormat;

    /// Returns the endpoint's total buffer size in frames.
    fn buffer_size_frames(&self) -> usize;

    /// Returns the backend's preferred render period in frames.
    fn period_frames(&self) -> usize;

    /// Returns measured stream latency in output frames, when available.
    fn estimated_latency_frames(&self) -> Option<u64>;

    /// Returns the complete control-plane observation for this output mode.
    ///
    /// The default preserves the existing Shared-mode evidence policy. An
    /// Exclusive backend may override this to report that the system mixer
    /// was bypassed, while still keeping all report construction off the
    /// realtime render path.
    fn output_observation(&self, request: OutputRequest) -> OutputObservation {
        observation_for(
            request,
            self.negotiated_format(),
            self.estimated_latency_frames(),
        )
    }

    /// Returns the current endpoint padding.
    ///
    /// # Errors
    ///
    /// Returns a backend error when the endpoint cannot report its padding.
    fn current_padding_frames(&mut self) -> Result<usize, RenderBackendError>;

    /// Waits for the event-driven scheduler signal. Waiting belongs to the
    /// scheduler, not the realtime PCM renderer.
    ///
    /// # Errors
    ///
    /// Returns a backend error when the event wait fails.
    fn wait_for_event(&mut self) -> Result<RenderEvent, RenderBackendError>;

    /// Starts the endpoint client.
    ///
    /// # Errors
    ///
    /// Returns a backend error when the client cannot start.
    fn start(&mut self) -> Result<(), RenderBackendError>;

    /// Stops the endpoint client.
    ///
    /// # Errors
    ///
    /// Returns a backend error when the client cannot stop.
    fn stop(&mut self) -> Result<(), RenderBackendError>;

    /// Copies interleaved `f32` samples into the already acquired endpoint
    /// buffer, performing only bounded, prevalidated sample conversion.
    ///
    /// # Errors
    ///
    /// Returns a backend error when the endpoint buffer cannot be acquired or
    /// the supplied frame/sample lengths are invalid.
    fn write_interleaved_f32(
        &mut self,
        frames: usize,
        samples: &[f32],
    ) -> Result<(), RenderBackendError>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OutputLifecycle {
    Prepared,
    Running,
    Unavailable,
    Stopped,
}

/// A generic event-driven Shared output. The generic backend is what makes
/// deterministic device/render tests possible without a physical endpoint.
#[derive(Debug)]
pub struct SharedOutput<B: SharedRenderBackend> {
    backend: B,
    fallback_policy: FallbackPolicy,
    report: OutputReport,
    scratch: Vec<f32>,
    period_frames: usize,
    lifecycle: OutputLifecycle,
}

impl<B: SharedRenderBackend> SharedOutput<B> {
    /// Builds a prepared output from a fully negotiated backend.
    ///
    /// Allocation is limited to this control-plane constructor. The scratch
    /// area is sized from the backend's fixed endpoint buffer and reused for
    /// every event-driven render period.
    ///
    /// # Errors
    ///
    /// Returns [`SharedOutputError::InvalidConfiguration`] when the backend
    /// reports an unusable buffer or period.
    pub fn from_backend(
        config: &SharedOutputConfig,
        backend: B,
    ) -> Result<Self, SharedOutputError> {
        let buffer_frames = backend.buffer_size_frames();
        let backend_period = backend.period_frames();
        let format = backend.negotiated_format();
        if buffer_frames == 0 {
            return Err(SharedOutputError::InvalidConfiguration {
                field: "buffer_size_frames",
            });
        }
        if backend_period == 0 {
            return Err(SharedOutputError::InvalidConfiguration {
                field: "period_frames",
            });
        }
        let period_frames = config
            .period_frames()
            .unwrap_or(backend_period)
            .min(buffer_frames);
        if period_frames == 0 {
            return Err(SharedOutputError::InvalidConfiguration {
                field: "period_frames",
            });
        }
        let samples = format.sample_count_for_frames(buffer_frames).ok_or(
            SharedOutputError::InvalidConfiguration {
                field: "buffer_size_frames",
            },
        )?;
        let scratch = vec![0.0; samples];
        let observation = backend.output_observation(config.request());
        Ok(Self {
            backend,
            fallback_policy: config.fallback_policy(),
            report: OutputReport::new(config.request(), observation),
            scratch,
            period_frames,
            lifecycle: OutputLifecycle::Prepared,
        })
    }

    /// Returns the immutable requested/observed output report.
    #[must_use]
    pub const fn report(&self) -> OutputReport {
        self.report
    }

    /// Returns the selected endpoint projection.
    #[must_use]
    pub fn device(&self) -> &DeviceInfo {
        self.backend.device()
    }

    /// Returns the negotiated endpoint format.
    #[must_use]
    pub fn negotiated_format(&self) -> AudioFormat {
        self.backend.negotiated_format()
    }

    /// Returns the fixed event period used by the render scheduler.
    #[must_use]
    pub const fn period_frames(&self) -> usize {
        self.period_frames
    }

    /// Starts the already negotiated endpoint.
    ///
    /// # Errors
    ///
    /// Returns a typed output error when the endpoint is unavailable or fails
    /// to start.
    pub fn start(&mut self) -> Result<(), SharedOutputError> {
        self.ensure_not_unavailable()?;
        self.backend
            .start()
            .map_err(|error| self.map_backend_error(error, false))?;
        self.lifecycle = OutputLifecycle::Running;
        Ok(())
    }

    /// Stops the endpoint. Stop is a control-plane operation and may be
    /// called after a render error to release endpoint resources.
    ///
    /// # Errors
    ///
    /// Returns a typed output error when the endpoint fails to stop.
    pub fn stop(&mut self) -> Result<(), SharedOutputError> {
        if matches!(self.lifecycle, OutputLifecycle::Stopped) {
            return Ok(());
        }
        self.backend
            .stop()
            .map_err(|error| self.map_backend_error(error, false))?;
        self.lifecycle = OutputLifecycle::Stopped;
        Ok(())
    }

    /// Waits for a scheduler event. This method may block and is intentionally
    /// separate from [`Self::render_event`], which is the bounded realtime
    /// operation.
    ///
    /// # Errors
    ///
    /// Returns a typed output error when the endpoint wait fails.
    pub fn wait_for_event(&mut self) -> Result<RenderEvent, SharedOutputError> {
        self.ensure_not_unavailable()?;
        self.backend
            .wait_for_event()
            .map_err(|error| self.map_backend_error(error, true))
    }

    /// Renders one event period into the endpoint buffer.
    ///
    /// The method performs no allocation, waits, locks, logging, database
    /// access, FFI calls, or UI dispatch. The backend has preallocated its
    /// endpoint resources and the caller supplies a Core-owned realtime
    /// renderer.
    ///
    /// # Errors
    ///
    /// Returns a typed output error when the endpoint becomes unavailable or
    /// rejects the render buffer.
    pub fn render_event(
        &mut self,
        renderer: &mut dyn RealtimePcmRenderer,
    ) -> Result<RenderCycle, SharedOutputError> {
        self.ensure_running()?;
        let padding = self
            .backend
            .current_padding_frames()
            .map_err(|error| self.map_backend_error(error, true))?;
        let capacity = self.backend.buffer_size_frames();
        let available = capacity.saturating_sub(padding).min(self.period_frames);
        if available == 0 {
            return Ok(RenderCycle {
                requested_frames: 0,
                outcome: RenderOutcome {
                    requested_frames: 0,
                    rendered_media_frames: 0,
                    silent_frames: 0,
                    discarded_stale_blocks: 0,
                    output_frame_aligned: true,
                },
            });
        }
        let channels = usize::from(self.negotiated_format().channels());
        let samples =
            available
                .checked_mul(channels)
                .ok_or(SharedOutputError::InvalidConfiguration {
                    field: "period_frames",
                })?;
        let output = &mut self.scratch[..samples];
        let outcome = renderer.render(output);
        self.backend
            .write_interleaved_f32(available, output)
            .map_err(|error| self.map_backend_error(error, true))?;
        Ok(RenderCycle {
            requested_frames: available,
            outcome,
        })
    }

    /// Handles one event without folding the potentially blocking wait into
    /// the realtime method.
    ///
    /// # Errors
    ///
    /// Returns a typed output error when waiting or rendering fails.
    pub fn pump_event(
        &mut self,
        renderer: &mut dyn RealtimePcmRenderer,
    ) -> Result<Option<RenderCycle>, SharedOutputError> {
        match self.wait_for_event()? {
            RenderEvent::Ready => self.render_event(renderer).map(Some),
            RenderEvent::Timeout => Ok(None),
        }
    }

    fn ensure_not_unavailable(&self) -> Result<(), SharedOutputError> {
        if matches!(self.lifecycle, OutputLifecycle::Unavailable) {
            return Err(SharedOutputError::Unavailable {
                reason: OutputUnavailableReason::DeviceRemoved,
                fallback: self.fallback_for(OutputFallback::DefaultDevice),
            });
        }
        Ok(())
    }

    fn ensure_running(&self) -> Result<(), SharedOutputError> {
        match self.lifecycle {
            OutputLifecycle::Running => Ok(()),
            OutputLifecycle::Unavailable => self.ensure_not_unavailable(),
            OutputLifecycle::Prepared => Err(SharedOutputError::InvalidConfiguration {
                field: "lifecycle (start is required)",
            }),
            OutputLifecycle::Stopped => Err(SharedOutputError::InvalidConfiguration {
                field: "lifecycle (output is stopped)",
            }),
        }
    }

    fn fallback_for(&self, fallback: OutputFallback) -> Option<OutputFallback> {
        matches!(self.fallback_policy, FallbackPolicy::DefaultDevice).then_some(fallback)
    }

    fn map_backend_error(
        &mut self,
        error: RenderBackendError,
        mark_unavailable: bool,
    ) -> SharedOutputError {
        match error {
            RenderBackendError::DeviceRemoved => {
                if mark_unavailable {
                    self.lifecycle = OutputLifecycle::Unavailable;
                }
                SharedOutputError::Unavailable {
                    reason: OutputUnavailableReason::DeviceRemoved,
                    fallback: self.fallback_for(OutputFallback::DefaultDevice),
                }
            }
            RenderBackendError::Unavailable => SharedOutputError::Unavailable {
                reason: OutputUnavailableReason::BackendUnavailable,
                fallback: self.fallback_for(OutputFallback::DefaultDevice),
            },
            RenderBackendError::Api { operation, hresult } => SharedOutputError::Backend {
                operation,
                hresult: Some(hresult),
                fallback: self.fallback_for(OutputFallback::DefaultDevice),
            },
            RenderBackendError::InvalidBuffer => SharedOutputError::Backend {
                operation: RenderOperation::AcquireBuffer,
                hresult: None,
                fallback: None,
            },
        }
    }
}

impl<B: SharedRenderBackend> Drop for SharedOutput<B> {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}

fn observation_for(
    request: OutputRequest,
    negotiated: AudioFormat,
    latency_frames: Option<u64>,
) -> OutputObservation {
    let source = request.source_format();
    let conversion = if source.sample_format() == negotiated.sample_format() {
        FormatConversionStatus::NotApplied
    } else {
        FormatConversionStatus::Applied
    };
    let resampling = if source.sample_rate_hz() == negotiated.sample_rate_hz() {
        ResamplingStatus::NotApplied
    } else {
        ResamplingStatus::Applied
    };
    let mapping = if source.channels() == negotiated.channels() {
        ChannelMappingStatus::Identity
    } else {
        ChannelMappingStatus::Remixed
    };
    // Shared-mode system effects are not proven absent by this adapter. The
    // unknown DSP evidence intentionally prevents a bit-perfect claim.
    OutputObservation::new(
        Some(negotiated),
        Some(PlaybackRate::NORMAL),
        Some(Volume::UNITY),
        latency_frames,
        conversion,
        resampling,
        mapping,
        None,
        Some(false),
    )
}

fn fallback_for(policy: FallbackPolicy, fallback: OutputFallback) -> Option<OutputFallback> {
    matches!(policy, FallbackPolicy::DefaultDevice).then_some(fallback)
}

#[cfg(all(windows, target_arch = "x86_64"))]
mod platform;

#[cfg(all(windows, target_arch = "x86_64"))]
pub use platform::{
    WasapiDeviceEnumerator, WasapiExclusiveBackend, WasapiExclusiveOutput, WasapiSharedBackend,
    WasapiSharedOutput, enumerate_devices, open_exclusive, open_shared,
};

#[cfg(not(windows))]
/// Windows output is unavailable on non-Windows hosts; deterministic tests
/// should use a fake [`SharedRenderBackend`].
///
/// # Errors
///
/// Always returns [`SharedOutputError::Unavailable`] on non-Windows hosts.
pub fn open_shared(_config: &SharedOutputConfig) -> Result<(), SharedOutputError> {
    Err(SharedOutputError::Unavailable {
        reason: OutputUnavailableReason::BackendUnavailable,
        fallback: None,
    })
}

#[cfg(not(windows))]
/// Exclusive output is unavailable on non-Windows hosts; deterministic tests
/// should use [`open_exclusive_with_shared_fallback`] with a fake factory.
///
/// # Errors
///
/// Always returns [`ExclusiveOutputError::Unavailable`].
pub fn open_exclusive(_config: &ExclusiveOutputConfig) -> Result<(), ExclusiveOutputError> {
    Err(ExclusiveOutputError::Unavailable {
        reason: OutputUnavailableReason::BackendUnavailable,
        fallback: None,
    })
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;

    use aurorix_audio::format::SampleFormat;

    use super::*;

    #[derive(Debug)]
    struct FakeBackend {
        device: DeviceInfo,
        format: AudioFormat,
        exclusive_evidence: bool,
        buffer_frames: usize,
        period_frames: usize,
        latency_frames: Option<u64>,
        padding_frames: usize,
        events: VecDeque<Result<RenderEvent, RenderBackendError>>,
        written: Vec<f32>,
        running: bool,
    }

    impl FakeBackend {
        fn new(format: AudioFormat) -> Self {
            Self {
                device: DeviceInfo::new("fake-device", "Fake DAC", DeviceState::Active, true)
                    .expect("fake id is valid"),
                format,
                exclusive_evidence: false,
                buffer_frames: 8,
                period_frames: 4,
                latency_frames: Some(32),
                padding_frames: 0,
                events: VecDeque::from([Ok(RenderEvent::Ready)]),
                written: Vec::new(),
                running: false,
            }
        }
    }

    impl SharedRenderBackend for FakeBackend {
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
            if self.exclusive_evidence {
                OutputObservation::new(
                    Some(self.format),
                    Some(PlaybackRate::NORMAL),
                    Some(Volume::UNITY),
                    self.latency_frames,
                    FormatConversionStatus::NotApplied,
                    ResamplingStatus::NotApplied,
                    ChannelMappingStatus::Identity,
                    Some(false),
                    Some(false),
                )
            } else {
                super::observation_for(request, self.format, self.latency_frames)
            }
        }

        fn current_padding_frames(&mut self) -> Result<usize, RenderBackendError> {
            Ok(self.padding_frames)
        }

        fn wait_for_event(&mut self) -> Result<RenderEvent, RenderBackendError> {
            self.events.pop_front().unwrap_or(Ok(RenderEvent::Timeout))
        }

        fn start(&mut self) -> Result<(), RenderBackendError> {
            self.running = true;
            Ok(())
        }

        fn stop(&mut self) -> Result<(), RenderBackendError> {
            self.running = false;
            Ok(())
        }

        fn write_interleaved_f32(
            &mut self,
            frames: usize,
            samples: &[f32],
        ) -> Result<(), RenderBackendError> {
            if samples.len() != frames * usize::from(self.format.channels()) {
                return Err(RenderBackendError::InvalidBuffer);
            }
            self.written.extend_from_slice(samples);
            Ok(())
        }
    }

    struct FixedRenderer;

    impl RealtimePcmRenderer for FixedRenderer {
        fn render(&mut self, output: &mut [f32]) -> RenderOutcome {
            for (index, sample) in output.iter_mut().enumerate() {
                *sample = f32::from(u16::try_from(index).expect("test output fits")) / 10.0;
            }
            let frames = output.len() / 2;
            RenderOutcome {
                requested_frames: frames,
                rendered_media_frames: frames,
                silent_frames: 0,
                discarded_stale_blocks: 0,
                output_frame_aligned: true,
            }
        }
    }

    fn request(format: AudioFormat) -> OutputRequest {
        OutputRequest::new(
            aurorix_audio::output_report::SourceCodec::Flac,
            format,
            Some(format),
            PlaybackRate::NORMAL,
            Volume::UNITY,
        )
    }

    #[derive(Debug)]
    struct FakeExclusiveFactory {
        support: ExclusiveFormatSupport,
        exclusive_query_error: Option<ExclusiveOutputError>,
        exclusive_error: Option<ExclusiveOutputError>,
        shared_error: Option<SharedOutputError>,
        shared_format: AudioFormat,
        exclusive_calls: usize,
        shared_calls: usize,
    }

    impl FakeExclusiveFactory {
        fn new(support: ExclusiveFormatSupport, shared_format: AudioFormat) -> Self {
            Self {
                support,
                exclusive_query_error: None,
                exclusive_error: None,
                shared_error: None,
                shared_format,
                exclusive_calls: 0,
                shared_calls: 0,
            }
        }

        fn prepared(
            config: &SharedOutputConfig,
            format: AudioFormat,
            exclusive_evidence: bool,
        ) -> Result<SharedOutput<FakeBackend>, SharedOutputError> {
            let mut backend = FakeBackend::new(format);
            backend.exclusive_evidence = exclusive_evidence;
            SharedOutput::from_backend(config, backend)
        }
    }

    impl ExclusiveBackendFactory for FakeExclusiveFactory {
        type ExclusiveBackend = FakeBackend;
        type SharedBackend = FakeBackend;

        fn exclusive_format_support(
            &mut self,
            _requested: AudioFormat,
        ) -> Result<ExclusiveFormatSupport, ExclusiveOutputError> {
            if let Some(error) = self.exclusive_query_error.take() {
                return Err(error);
            }
            Ok(self.support)
        }

        fn open_exclusive(
            &mut self,
            config: &ExclusiveOutputConfig,
            negotiated_format: AudioFormat,
        ) -> Result<SharedOutput<Self::ExclusiveBackend>, ExclusiveOutputError> {
            self.exclusive_calls += 1;
            if let Some(error) = self.exclusive_error.take() {
                return Err(error);
            }
            Self::prepared(&config.shared_config(), negotiated_format, true).map_err(|error| {
                ExclusiveOutputError::InvalidConfiguration {
                    field: match error {
                        SharedOutputError::InvalidConfiguration { field } => field,
                        _ => "fake backend",
                    },
                }
            })
        }

        fn open_shared(
            &mut self,
            config: &SharedOutputConfig,
        ) -> Result<SharedOutput<Self::SharedBackend>, SharedOutputError> {
            self.shared_calls += 1;
            if let Some(error) = self.shared_error.take() {
                return Err(error);
            }
            Self::prepared(config, self.shared_format, false)
        }
    }

    #[test]
    fn exclusive_exact_request_reports_actual_format_and_complete_evidence() {
        let format = AudioFormat::new(96_000, 2, SampleFormat::I24).expect("format is valid");
        let config = ExclusiveOutputConfig::new(request(format));
        let mut factory = FakeExclusiveFactory::new(ExclusiveFormatSupport::Exact, format);
        let output = open_exclusive_with_shared_fallback(&mut factory, &config)
            .expect("exact Exclusive request prepares");

        let ExclusiveOpenOutput::Exclusive(output) = output else {
            panic!("exact support must not fall back");
        };
        assert_eq!(output.negotiated_format(), format);
        assert_eq!(output.report().negotiated_output_format(), Some(format));
        assert!(output.report().bit_perfect_eligible());
        assert_eq!(factory.exclusive_calls, 1);
        assert_eq!(factory.shared_calls, 0);
    }

    #[test]
    fn exclusive_backend_failure_uses_typed_shared_fallback() {
        let format = AudioFormat::f32(48_000, 2).expect("format is valid");
        let config = ExclusiveOutputConfig::new(request(format));
        let mut factory = FakeExclusiveFactory::new(ExclusiveFormatSupport::Exact, format);
        factory.exclusive_error = Some(ExclusiveOutputError::Backend {
            operation: RenderOperation::Initialize,
            hresult: Some(-2),
            fallback: Some(OutputFallback::SharedMode),
        });

        let output = open_exclusive_with_shared_fallback(&mut factory, &config)
            .expect("Shared fallback handles Exclusive setup failure");
        assert!(matches!(output, ExclusiveOpenOutput::SharedFallback(_)));
        assert_eq!(factory.exclusive_calls, 1);
        assert_eq!(factory.shared_calls, 1);
    }

    #[test]
    fn closest_exclusive_format_is_reported_separately_from_request() {
        let requested = AudioFormat::new(96_000, 2, SampleFormat::I24).expect("format is valid");
        let closest = AudioFormat::f32(48_000, 2).expect("format is valid");
        let config = ExclusiveOutputConfig::new(request(requested));
        let mut factory =
            FakeExclusiveFactory::new(ExclusiveFormatSupport::Closest(closest), closest);
        let output = open_exclusive_with_shared_fallback(&mut factory, &config)
            .expect("closest Exclusive format prepares");

        let ExclusiveOpenOutput::Exclusive(output) = output else {
            panic!("closest support is still an Exclusive success");
        };
        assert_eq!(output.report().requested_output_format(), Some(requested));
        assert_eq!(output.report().negotiated_output_format(), Some(closest));
        assert!(!output.report().bit_perfect_eligible());
    }

    #[test]
    fn unsupported_exclusive_format_uses_typed_shared_fallback() {
        let requested = AudioFormat::new(96_000, 2, SampleFormat::I24).expect("format is valid");
        let shared = AudioFormat::f32(48_000, 2).expect("format is valid");
        let config = ExclusiveOutputConfig::new(request(requested));
        let mut factory = FakeExclusiveFactory::new(ExclusiveFormatSupport::Unsupported, shared);
        let output = open_exclusive_with_shared_fallback(&mut factory, &config)
            .expect("Shared fallback prepares");

        let ExclusiveOpenOutput::SharedFallback(output) = output else {
            panic!("unsupported Exclusive format must use Shared fallback");
        };
        assert_eq!(output.negotiated_format(), shared);
        assert_eq!(output.report().observation().dsp_enabled(), None);
        assert!(!output.report().bit_perfect_eligible());
        assert_eq!(factory.exclusive_calls, 0);
        assert_eq!(factory.shared_calls, 1);
    }

    #[test]
    fn exclusive_format_query_failure_uses_typed_shared_fallback() {
        let format = AudioFormat::f32(48_000, 2).expect("format is valid");
        let config = ExclusiveOutputConfig::new(request(format));
        let mut factory = FakeExclusiveFactory::new(ExclusiveFormatSupport::Exact, format);
        factory.exclusive_query_error = Some(ExclusiveOutputError::Backend {
            operation: RenderOperation::NegotiateFormat,
            hresult: Some(-1),
            fallback: Some(OutputFallback::SharedMode),
        });

        let output = open_exclusive_with_shared_fallback(&mut factory, &config)
            .expect("Shared fallback handles a typed Exclusive query failure");
        assert!(matches!(output, ExclusiveOpenOutput::SharedFallback(_)));
        assert_eq!(factory.exclusive_calls, 0);
        assert_eq!(factory.shared_calls, 1);
    }

    #[test]
    fn disabled_exclusive_fallback_returns_original_typed_error_without_retry() {
        let format = AudioFormat::f32(48_000, 2).expect("format is valid");
        let config = ExclusiveOutputConfig::new(request(format))
            .with_fallback_policy(FallbackPolicy::Disabled);
        let mut factory = FakeExclusiveFactory::new(ExclusiveFormatSupport::Unsupported, format);
        let error = open_exclusive_with_shared_fallback(&mut factory, &config)
            .expect_err("disabled fallback must preserve the Exclusive error");

        assert_eq!(factory.shared_calls, 0);
        assert_eq!(
            error,
            ExclusiveOpenError::Exclusive(ExclusiveOutputError::FormatNegotiation {
                requested: format,
                closest: None,
                fallback: None,
            })
        );
    }

    #[test]
    fn shared_fallback_failure_preserves_both_typed_failures() {
        let format = AudioFormat::f32(48_000, 2).expect("format is valid");
        let config = ExclusiveOutputConfig::new(request(format));
        let mut factory = FakeExclusiveFactory::new(ExclusiveFormatSupport::Unsupported, format);
        factory.shared_error = Some(SharedOutputError::Unavailable {
            reason: OutputUnavailableReason::BackendUnavailable,
            fallback: None,
        });
        let error = open_exclusive_with_shared_fallback(&mut factory, &config)
            .expect_err("failed Shared retry is returned as a typed composite error");

        assert_eq!(
            error,
            ExclusiveOpenError::SharedFallback {
                exclusive: ExclusiveOutputError::FormatNegotiation {
                    requested: format,
                    closest: None,
                    fallback: Some(OutputFallback::SharedMode),
                },
                shared: SharedOutputError::Unavailable {
                    reason: OutputUnavailableReason::BackendUnavailable,
                    fallback: None,
                },
            }
        );
        assert_eq!(factory.shared_calls, 1);
    }

    #[test]
    fn fake_shared_output_negotiates_report_and_renders_one_event() {
        let format = AudioFormat::f32(48_000, 2).expect("format is valid");
        let backend = FakeBackend::new(format);
        let mut output =
            SharedOutput::from_backend(&SharedOutputConfig::new(request(format)), backend)
                .expect("fake output prepares");
        assert_eq!(output.negotiated_format(), format);
        assert_eq!(output.report().estimated_latency_frames(), Some(32));
        assert!(!output.report().bit_perfect_eligible());
        assert_eq!(output.report().observation().dsp_enabled(), None);

        output.start().expect("fake output starts");
        let mut renderer = FixedRenderer;
        let cycle = output
            .pump_event(&mut renderer)
            .expect("event render succeeds")
            .expect("fake event is ready");
        assert_eq!(cycle.requested_frames, 4);
        assert_eq!(cycle.outcome.rendered_media_frames, 4);
        assert_eq!(output.backend.written.len(), 8);
        assert_eq!(output.backend.written[3].to_bits(), 0.3_f32.to_bits());
    }

    #[test]
    fn fake_device_removal_is_typed_and_advertises_default_fallback() {
        let format = AudioFormat::f32(48_000, 2).expect("format is valid");
        let mut backend = FakeBackend::new(format);
        backend.events = VecDeque::from([Err(RenderBackendError::DeviceRemoved)]);
        let mut output =
            SharedOutput::from_backend(&SharedOutputConfig::new(request(format)), backend)
                .expect("fake output prepares");
        output.start().expect("fake output starts");
        let error = output
            .wait_for_event()
            .expect_err("device removal is returned");
        assert_eq!(
            error,
            SharedOutputError::Unavailable {
                reason: OutputUnavailableReason::DeviceRemoved,
                fallback: Some(OutputFallback::DefaultDevice),
            }
        );
        assert!(matches!(
            output.wait_for_event(),
            Err(SharedOutputError::Unavailable {
                reason: OutputUnavailableReason::DeviceRemoved,
                ..
            })
        ));
    }

    #[test]
    fn disabled_fallback_does_not_offer_automatic_retry() {
        let format = AudioFormat::f32(48_000, 2).expect("format is valid");
        let mut backend = FakeBackend::new(format);
        backend.events = VecDeque::from([Err(RenderBackendError::DeviceRemoved)]);
        let config =
            SharedOutputConfig::new(request(format)).with_fallback_policy(FallbackPolicy::Disabled);
        let mut output = SharedOutput::from_backend(&config, backend).expect("output prepares");
        output.start().expect("fake output starts");
        let error = output
            .wait_for_event()
            .expect_err("device removal is returned");
        assert_eq!(
            error,
            SharedOutputError::Unavailable {
                reason: OutputUnavailableReason::DeviceRemoved,
                fallback: None,
            }
        );
    }

    #[test]
    fn device_selector_and_format_conversion_are_control_plane_values() {
        let id = DeviceId::new("endpoint").expect("id is valid");
        let config = SharedOutputConfig::new(request(
            AudioFormat::new(96_000, 2, SampleFormat::I24).expect("format is valid"),
        ))
        .with_device(DeviceSelector::Id(id.clone()))
        .with_period_frames(Some(3));
        assert_eq!(config.selector(), &DeviceSelector::Id(id));
        assert_eq!(config.period_frames(), Some(3));

        let source = config.request().source_format();
        let negotiated = AudioFormat::f32(48_000, 2).expect("format is valid");
        let observation = observation_for(config.request(), negotiated, Some(1));
        assert_eq!(
            observation.format_conversion(),
            FormatConversionStatus::Applied
        );
        assert_eq!(observation.resampling(), ResamplingStatus::Applied);
        assert!(!OutputReport::new(config.request(), observation).bit_perfect_eligible());
        assert_eq!(source.sample_format(), SampleFormat::I24);
    }
}
