//! Requested versus observed output capability reporting.
//!
//! A requested format, rate, or volume is intent only. Bit-perfect eligibility
//! is derived exclusively from validated source data and observed output
//! evidence. Unknown observed values conservatively disable the claim.

use std::{error::Error, fmt};

use super::format::AudioFormat;

/// The release-one codec families understood by the local playback contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SourceCodec {
    /// PCM samples in a RIFF/WAV container.
    WavPcm,
    /// Free Lossless Audio Codec.
    Flac,
    /// MPEG-1/2 Layer III.
    Mp3,
    /// Low Complexity AAC in M4A or ADTS.
    AacLc,
    /// Opus in an Ogg container.
    OpusOgg,
    /// A codec outside the release-one matrix.
    Unknown,
}

/// A fixed-point playback rate measured in millionths of normal speed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PlaybackRate(u32);

impl PlaybackRate {
    /// Normal 1.0x playback.
    pub const NORMAL: Self = Self(1_000_000);

    /// Creates a positive fixed-point playback rate.
    ///
    /// # Errors
    ///
    /// Returns [`OutputReportError::InvalidPlaybackRate`] for zero.
    pub const fn from_millionths(value: u32) -> Result<Self, OutputReportError> {
        if value == 0 {
            return Err(OutputReportError::InvalidPlaybackRate);
        }
        Ok(Self(value))
    }

    /// Returns the millionths-of-normal representation.
    #[must_use]
    pub const fn as_millionths(self) -> u32 {
        self.0
    }
}

/// A normalized output volume with an explicit mute bit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Volume {
    level_millionths: u32,
    muted: bool,
}

impl Volume {
    /// Unity, unmuted volume.
    pub const UNITY: Self = Self {
        level_millionths: 1_000_000,
        muted: false,
    };

    /// Creates a volume in the inclusive normalized range 0..=1.
    ///
    /// # Errors
    ///
    /// Returns [`OutputReportError::InvalidVolume`] when the level exceeds
    /// one millionths of unity.
    pub const fn new(level_millionths: u32, muted: bool) -> Result<Self, OutputReportError> {
        if level_millionths > 1_000_000 {
            return Err(OutputReportError::InvalidVolume);
        }
        Ok(Self {
            level_millionths,
            muted,
        })
    }

    /// Returns the normalized level in millionths.
    #[must_use]
    pub const fn level_millionths(self) -> u32 {
        self.level_millionths
    }

    /// Returns whether output is muted.
    #[must_use]
    pub const fn is_muted(self) -> bool {
        self.muted
    }

    /// Returns whether this is unity, unmuted volume.
    #[must_use]
    pub const fn is_unity(self) -> bool {
        self.level_millionths == 1_000_000 && !self.muted
    }
}

/// Whether the observed path changed the source sample rate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ResamplingStatus {
    /// The observed path did not resample.
    NotApplied,
    /// The observed path resampled at least once.
    Applied,
    /// The adapter did not provide enough evidence.
    Unknown,
}

/// Whether the output path converted the sample representation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FormatConversionStatus {
    /// The source sample representation was preserved.
    NotApplied,
    /// The path converted the source sample representation.
    Applied,
    /// The adapter did not provide sample-conversion evidence.
    Unknown,
}

/// A requested processing preference. This is caller intent, not evidence
/// that the output adapter applied or omitted the processing stage.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ProcessingPreference {
    /// The caller requests that the stage remain disabled.
    Disabled,
    /// The caller requests or permits the stage.
    Enabled,
}

/// Whether observed channels remained identity-mapped.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ChannelMappingStatus {
    /// Every source channel reached the same corresponding output channel.
    Identity,
    /// The path remixed, downmixed, or upmixed channels.
    Remixed,
    /// The adapter did not provide channel-layout evidence.
    Unknown,
}

/// The reason a report cannot make a strict bit-perfect claim.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BitPerfectReason {
    /// All required evidence was present and compatible.
    Eligible,
    /// No observed negotiated format was supplied.
    MissingObservedFormat,
    /// No measured output latency was supplied with the negotiated output.
    MissingObservedLatency,
    /// The source codec is outside the release-one evidence set.
    UnknownSourceCodec,
    /// The observed format differs from the source format.
    FormatChanged,
    /// The observed playback rate is unknown or not normal.
    PlaybackRateChanged,
    /// The requested or observed volume is not unity and unmuted.
    VolumeProcessing,
    /// The path resampled or did not report resampling status.
    Resampling,
    /// The path converted the sample representation or did not report it.
    FormatConversion,
    /// The path remixed or did not report channel mapping status.
    ChannelMapping,
    /// A DSP graph is enabled.
    DspEnabled,
    /// The adapter did not report whether DSP was enabled.
    MissingDspEvidence,
    /// Crossfade mixed more than one graph.
    CrossfadeEnabled,
    /// The adapter did not report whether crossfade was enabled.
    MissingCrossfadeEvidence,
}

/// Requested output intent supplied before an adapter negotiates output.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct OutputRequest {
    source_codec: SourceCodec,
    source_format: AudioFormat,
    requested_output_format: Option<AudioFormat>,
    playback_rate: PlaybackRate,
    volume: Volume,
    requested_latency_frames: Option<u64>,
    resampling: ProcessingPreference,
    channel_mapping: ProcessingPreference,
    dsp: ProcessingPreference,
    crossfade: ProcessingPreference,
}

impl OutputRequest {
    /// Creates a validated output request.
    #[must_use]
    pub const fn new(
        source_codec: SourceCodec,
        source_format: AudioFormat,
        requested_output_format: Option<AudioFormat>,
        playback_rate: PlaybackRate,
        volume: Volume,
    ) -> Self {
        Self {
            source_codec,
            source_format,
            requested_output_format,
            playback_rate,
            volume,
            requested_latency_frames: None,
            resampling: ProcessingPreference::Disabled,
            channel_mapping: ProcessingPreference::Disabled,
            dsp: ProcessingPreference::Disabled,
            crossfade: ProcessingPreference::Disabled,
        }
    }

    /// Adds requested latency and processing intent to this output request.
    ///
    /// These values remain separate from adapter observations and never
    /// establish a quality claim on their own.
    #[must_use]
    pub const fn with_processing_preferences(
        mut self,
        requested_latency_frames: Option<u64>,
        resampling: ProcessingPreference,
        channel_mapping: ProcessingPreference,
        dsp: ProcessingPreference,
        crossfade: ProcessingPreference,
    ) -> Self {
        self.requested_latency_frames = requested_latency_frames;
        self.resampling = resampling;
        self.channel_mapping = channel_mapping;
        self.dsp = dsp;
        self.crossfade = crossfade;
        self
    }

    /// Returns the requested codec family.
    #[must_use]
    pub const fn source_codec(self) -> SourceCodec {
        self.source_codec
    }

    /// Returns the validated source format.
    #[must_use]
    pub const fn source_format(self) -> AudioFormat {
        self.source_format
    }

    /// Returns the requested output format, when one was specified.
    #[must_use]
    pub const fn requested_output_format(self) -> Option<AudioFormat> {
        self.requested_output_format
    }

    /// Returns the requested playback rate.
    #[must_use]
    pub const fn playback_rate(self) -> PlaybackRate {
        self.playback_rate
    }

    /// Returns the requested volume.
    #[must_use]
    pub const fn volume(self) -> Volume {
        self.volume
    }

    /// Returns the requested output-latency target, when supplied.
    #[must_use]
    pub const fn requested_latency_frames(self) -> Option<u64> {
        self.requested_latency_frames
    }

    /// Returns the requested resampling preference.
    #[must_use]
    pub const fn resampling(self) -> ProcessingPreference {
        self.resampling
    }

    /// Returns the requested channel-mapping preference.
    #[must_use]
    pub const fn channel_mapping(self) -> ProcessingPreference {
        self.channel_mapping
    }

    /// Returns the requested DSP preference.
    #[must_use]
    pub const fn dsp(self) -> ProcessingPreference {
        self.dsp
    }

    /// Returns the requested crossfade preference.
    #[must_use]
    pub const fn crossfade(self) -> ProcessingPreference {
        self.crossfade
    }
}

/// Observed facts supplied by an output or capture adapter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct OutputObservation {
    negotiated_output_format: Option<AudioFormat>,
    playback_rate: Option<PlaybackRate>,
    volume: Option<Volume>,
    estimated_latency_frames: Option<u64>,
    format_conversion: FormatConversionStatus,
    resampling: ResamplingStatus,
    channel_mapping: ChannelMappingStatus,
    dsp_enabled: Option<bool>,
    crossfade_enabled: Option<bool>,
}

impl OutputObservation {
    /// Creates an observation with explicit known or unknown adapter facts.
    #[must_use]
    #[allow(clippy::too_many_arguments)]
    pub const fn new(
        negotiated_output_format: Option<AudioFormat>,
        playback_rate: Option<PlaybackRate>,
        volume: Option<Volume>,
        estimated_latency_frames: Option<u64>,
        format_conversion: FormatConversionStatus,
        resampling: ResamplingStatus,
        channel_mapping: ChannelMappingStatus,
        dsp_enabled: Option<bool>,
        crossfade_enabled: Option<bool>,
    ) -> Self {
        Self {
            negotiated_output_format,
            playback_rate,
            volume,
            estimated_latency_frames,
            format_conversion,
            resampling,
            channel_mapping,
            dsp_enabled,
            crossfade_enabled,
        }
    }

    /// Creates a complete no-processing observation for deterministic tests.
    #[must_use]
    pub const fn direct(
        negotiated_output_format: AudioFormat,
        estimated_latency_frames: u64,
    ) -> Self {
        Self::new(
            Some(negotiated_output_format),
            Some(PlaybackRate::NORMAL),
            Some(Volume::UNITY),
            Some(estimated_latency_frames),
            FormatConversionStatus::NotApplied,
            ResamplingStatus::NotApplied,
            ChannelMappingStatus::Identity,
            Some(false),
            Some(false),
        )
    }

    /// Returns the observed negotiated format.
    #[must_use]
    pub const fn negotiated_output_format(self) -> Option<AudioFormat> {
        self.negotiated_output_format
    }

    /// Returns the observed playback rate.
    #[must_use]
    pub const fn playback_rate(self) -> Option<PlaybackRate> {
        self.playback_rate
    }

    /// Returns the observed volume.
    #[must_use]
    pub const fn volume(self) -> Option<Volume> {
        self.volume
    }

    /// Returns the observed output latency, when measured.
    #[must_use]
    pub const fn estimated_latency_frames(self) -> Option<u64> {
        self.estimated_latency_frames
    }

    /// Returns the observed sample-representation conversion state.
    #[must_use]
    pub const fn format_conversion(self) -> FormatConversionStatus {
        self.format_conversion
    }

    /// Returns the observed resampling status.
    #[must_use]
    pub const fn resampling(self) -> ResamplingStatus {
        self.resampling
    }

    /// Returns the observed channel mapping status.
    #[must_use]
    pub const fn channel_mapping(self) -> ChannelMappingStatus {
        self.channel_mapping
    }

    /// Returns whether a DSP stage is active.
    #[must_use]
    pub const fn dsp_enabled(self) -> Option<bool> {
        self.dsp_enabled
    }

    /// Returns whether crossfade mixing is active.
    #[must_use]
    pub const fn crossfade_enabled(self) -> Option<bool> {
        self.crossfade_enabled
    }
}

/// The immutable output report exposed to Core consumers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct OutputReport {
    request: OutputRequest,
    observation: OutputObservation,
    bit_perfect_reason: BitPerfectReason,
}

impl OutputReport {
    /// Builds a report and derives bit-perfect eligibility from observed facts.
    #[must_use]
    pub fn new(request: OutputRequest, observation: OutputObservation) -> Self {
        let bit_perfect_reason = derive_bit_perfect_reason(request, observation);
        Self {
            request,
            observation,
            bit_perfect_reason,
        }
    }

    /// Returns the requested portion of the report.
    #[must_use]
    pub const fn request(self) -> OutputRequest {
        self.request
    }

    /// Returns the observed portion of the report.
    #[must_use]
    pub const fn observation(self) -> OutputObservation {
        self.observation
    }

    /// Returns the source codec.
    #[must_use]
    pub const fn source_codec(self) -> SourceCodec {
        self.request.source_codec()
    }

    /// Returns the source format.
    #[must_use]
    pub const fn source_format(self) -> AudioFormat {
        self.request.source_format()
    }

    /// Returns the requested output format.
    #[must_use]
    pub const fn requested_output_format(self) -> Option<AudioFormat> {
        self.request.requested_output_format()
    }

    /// Returns the observed negotiated output format.
    #[must_use]
    pub const fn negotiated_output_format(self) -> Option<AudioFormat> {
        self.observation.negotiated_output_format()
    }

    /// Returns the requested playback rate.
    #[must_use]
    pub const fn playback_rate_requested(self) -> PlaybackRate {
        self.request.playback_rate()
    }

    /// Returns the observed playback rate.
    #[must_use]
    pub const fn playback_rate_actual(self) -> Option<PlaybackRate> {
        self.observation.playback_rate()
    }

    /// Returns the requested volume.
    #[must_use]
    pub const fn volume_requested(self) -> Volume {
        self.request.volume()
    }

    /// Returns the observed volume.
    #[must_use]
    pub const fn volume_actual(self) -> Option<Volume> {
        self.observation.volume()
    }

    /// Returns the measured output latency.
    #[must_use]
    pub const fn estimated_latency_frames(self) -> Option<u64> {
        self.observation.estimated_latency_frames()
    }

    /// Returns the requested output-latency target, when supplied.
    #[must_use]
    pub const fn requested_latency_frames(self) -> Option<u64> {
        self.request.requested_latency_frames()
    }

    /// Returns the requested resampling preference.
    #[must_use]
    pub const fn resampling_requested(self) -> ProcessingPreference {
        self.request.resampling()
    }

    /// Returns the requested channel-mapping preference.
    #[must_use]
    pub const fn channel_mapping_requested(self) -> ProcessingPreference {
        self.request.channel_mapping()
    }

    /// Returns the requested DSP preference.
    #[must_use]
    pub const fn dsp_requested(self) -> ProcessingPreference {
        self.request.dsp()
    }

    /// Returns the requested crossfade preference.
    #[must_use]
    pub const fn crossfade_requested(self) -> ProcessingPreference {
        self.request.crossfade()
    }

    /// Returns the observed sample-representation conversion state.
    #[must_use]
    pub const fn format_conversion(self) -> FormatConversionStatus {
        self.observation.format_conversion()
    }

    /// Returns the observed resampling state.
    #[must_use]
    pub const fn resampling(self) -> ResamplingStatus {
        self.observation.resampling()
    }

    /// Returns the observed channel mapping state.
    #[must_use]
    pub const fn channel_mapping(self) -> ChannelMappingStatus {
        self.observation.channel_mapping()
    }

    /// Returns whether DSP is active in the observed path.
    #[must_use]
    pub const fn dsp_enabled(self) -> Option<bool> {
        self.observation.dsp_enabled()
    }

    /// Returns whether crossfade is active in the observed path.
    #[must_use]
    pub const fn crossfade_enabled(self) -> Option<bool> {
        self.observation.crossfade_enabled()
    }

    /// Returns whether strict bit-perfect eligibility is proven.
    #[must_use]
    pub const fn bit_perfect_eligible(self) -> bool {
        matches!(self.bit_perfect_reason, BitPerfectReason::Eligible)
    }

    /// Returns the evidence reason behind the bit-perfect result.
    #[must_use]
    pub const fn bit_perfect_reason(self) -> BitPerfectReason {
        self.bit_perfect_reason
    }
}

/// A rejected requested output value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputReportError {
    /// A zero playback rate cannot describe output.
    InvalidPlaybackRate,
    /// A volume above unity is outside the normalized contract.
    InvalidVolume,
}

impl fmt::Display for OutputReportError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidPlaybackRate => formatter.write_str("playback rate must be non-zero"),
            Self::InvalidVolume => {
                formatter.write_str("volume must be within the normalized range")
            }
        }
    }
}

impl Error for OutputReportError {}

fn derive_bit_perfect_reason(
    request: OutputRequest,
    observation: OutputObservation,
) -> BitPerfectReason {
    let Some(observed_format) = observation.negotiated_output_format else {
        return BitPerfectReason::MissingObservedFormat;
    };
    if observation.estimated_latency_frames.is_none() {
        return BitPerfectReason::MissingObservedLatency;
    }
    if request.source_codec == SourceCodec::Unknown {
        return BitPerfectReason::UnknownSourceCodec;
    }
    if observed_format != request.source_format {
        return BitPerfectReason::FormatChanged;
    }
    if observation.playback_rate != Some(PlaybackRate::NORMAL)
        || request.playback_rate != PlaybackRate::NORMAL
    {
        return BitPerfectReason::PlaybackRateChanged;
    }
    if !request.volume.is_unity() || observation.volume != Some(Volume::UNITY) {
        return BitPerfectReason::VolumeProcessing;
    }
    if observation.resampling != ResamplingStatus::NotApplied {
        return BitPerfectReason::Resampling;
    }
    if observation.format_conversion != FormatConversionStatus::NotApplied {
        return BitPerfectReason::FormatConversion;
    }
    if observation.channel_mapping != ChannelMappingStatus::Identity {
        return BitPerfectReason::ChannelMapping;
    }
    match observation.dsp_enabled {
        Some(true) => return BitPerfectReason::DspEnabled,
        Some(false) => {}
        None => return BitPerfectReason::MissingDspEvidence,
    }
    match observation.crossfade_enabled {
        Some(true) => return BitPerfectReason::CrossfadeEnabled,
        Some(false) => {}
        None => return BitPerfectReason::MissingCrossfadeEvidence,
    }
    BitPerfectReason::Eligible
}

#[cfg(test)]
mod tests {
    use super::{
        BitPerfectReason, ChannelMappingStatus, FormatConversionStatus, OutputObservation,
        OutputReport, OutputRequest, PlaybackRate, ProcessingPreference, ResamplingStatus,
        SourceCodec, Volume,
    };
    use crate::format::{AudioFormat, SampleFormat};

    fn source_format() -> AudioFormat {
        AudioFormat::new(96_000, 2, SampleFormat::I24).expect("format is valid")
    }

    fn request() -> OutputRequest {
        OutputRequest::new(
            SourceCodec::Flac,
            source_format(),
            Some(source_format()),
            PlaybackRate::NORMAL,
            Volume::UNITY,
        )
    }

    #[test]
    fn requested_and_observed_values_are_separate() {
        let observed_format = AudioFormat::f32(48_000, 2).expect("format is valid");
        let requested = request().with_processing_preferences(
            Some(256),
            ProcessingPreference::Enabled,
            ProcessingPreference::Enabled,
            ProcessingPreference::Enabled,
            ProcessingPreference::Enabled,
        );
        let observed = OutputObservation::new(
            Some(observed_format),
            PlaybackRate::from_millionths(900_000).ok(),
            Volume::new(500_000, false).ok(),
            Some(128),
            FormatConversionStatus::Applied,
            ResamplingStatus::Applied,
            ChannelMappingStatus::Remixed,
            Some(true),
            Some(false),
        );
        let report = OutputReport::new(requested, observed);

        assert_eq!(report.source_format(), source_format());
        assert_eq!(report.negotiated_output_format(), Some(observed_format));
        assert_eq!(report.playback_rate_requested(), PlaybackRate::NORMAL);
        assert_eq!(
            report.playback_rate_actual(),
            PlaybackRate::from_millionths(900_000).ok()
        );
        assert_eq!(report.volume_requested(), Volume::UNITY);
        assert_eq!(report.volume_actual(), Volume::new(500_000, false).ok());
        assert_eq!(report.requested_latency_frames(), Some(256));
        assert_eq!(report.dsp_requested(), ProcessingPreference::Enabled);
        assert_eq!(report.dsp_enabled(), Some(true));
        assert!(!report.bit_perfect_eligible());
    }

    #[test]
    fn direct_observed_identity_is_eligible_only_with_complete_evidence() {
        let report = OutputReport::new(request(), OutputObservation::direct(source_format(), 64));

        assert!(report.bit_perfect_eligible());
        assert_eq!(report.bit_perfect_reason(), BitPerfectReason::Eligible);
        assert_eq!(report.estimated_latency_frames(), Some(64));
    }

    #[test]
    fn missing_observed_format_cannot_be_inferred_from_request() {
        let report = OutputReport::new(
            request(),
            OutputObservation::new(
                None,
                Some(PlaybackRate::NORMAL),
                Some(Volume::UNITY),
                None,
                FormatConversionStatus::Unknown,
                ResamplingStatus::Unknown,
                ChannelMappingStatus::Unknown,
                Some(false),
                Some(false),
            ),
        );

        assert!(!report.bit_perfect_eligible());
        assert_eq!(
            report.bit_perfect_reason(),
            BitPerfectReason::MissingObservedFormat
        );
    }

    #[test]
    fn f32_conversion_and_resampling_disable_bit_perfect() {
        let observed_format = AudioFormat::f32(96_000, 2).expect("format is valid");
        let report = OutputReport::new(
            request(),
            OutputObservation::new(
                Some(observed_format),
                Some(PlaybackRate::NORMAL),
                Some(Volume::UNITY),
                Some(32),
                FormatConversionStatus::Applied,
                ResamplingStatus::Applied,
                ChannelMappingStatus::Identity,
                Some(false),
                Some(false),
            ),
        );

        assert!(!report.bit_perfect_eligible());
        assert_eq!(report.bit_perfect_reason(), BitPerfectReason::FormatChanged);
    }

    #[test]
    fn dsp_crossfade_and_remix_are_explicit_negative_evidence() {
        let cases = [
            (
                ChannelMappingStatus::Identity,
                false,
                true,
                BitPerfectReason::CrossfadeEnabled,
            ),
            (
                ChannelMappingStatus::Identity,
                true,
                false,
                BitPerfectReason::DspEnabled,
            ),
            (
                ChannelMappingStatus::Remixed,
                false,
                false,
                BitPerfectReason::ChannelMapping,
            ),
        ];
        for (mapping, dsp, crossfade, reason) in cases {
            let report = OutputReport::new(
                request(),
                OutputObservation::new(
                    Some(source_format()),
                    Some(PlaybackRate::NORMAL),
                    Some(Volume::UNITY),
                    Some(16),
                    FormatConversionStatus::NotApplied,
                    ResamplingStatus::NotApplied,
                    mapping,
                    Some(dsp),
                    Some(crossfade),
                ),
            );
            assert!(!report.bit_perfect_eligible());
            assert_eq!(report.bit_perfect_reason(), reason);
        }
    }

    #[test]
    fn rate_and_volume_types_reject_invalid_values() {
        assert!(PlaybackRate::from_millionths(0).is_err());
        assert!(Volume::new(1_000_001, false).is_err());
    }

    #[test]
    fn incomplete_or_unknown_processing_evidence_is_not_eligible() {
        let cases = [
            (
                OutputObservation::new(
                    Some(source_format()),
                    Some(PlaybackRate::NORMAL),
                    Some(Volume::UNITY),
                    None,
                    FormatConversionStatus::NotApplied,
                    ResamplingStatus::NotApplied,
                    ChannelMappingStatus::Identity,
                    Some(false),
                    Some(false),
                ),
                BitPerfectReason::MissingObservedLatency,
            ),
            (
                OutputObservation::new(
                    Some(source_format()),
                    Some(PlaybackRate::NORMAL),
                    Some(Volume::UNITY),
                    Some(1),
                    FormatConversionStatus::Unknown,
                    ResamplingStatus::NotApplied,
                    ChannelMappingStatus::Identity,
                    Some(false),
                    Some(false),
                ),
                BitPerfectReason::FormatConversion,
            ),
        ];

        for (observation, reason) in cases {
            let report = OutputReport::new(request(), observation);
            assert!(!report.bit_perfect_eligible());
            assert_eq!(report.bit_perfect_reason(), reason);
        }
    }

    #[test]
    fn unknown_dsp_or_crossfade_evidence_cannot_pass() {
        let dsp_unknown = OutputReport::new(
            request(),
            OutputObservation::new(
                Some(source_format()),
                Some(PlaybackRate::NORMAL),
                Some(Volume::UNITY),
                Some(1),
                FormatConversionStatus::NotApplied,
                ResamplingStatus::NotApplied,
                ChannelMappingStatus::Identity,
                None,
                Some(false),
            ),
        );
        let crossfade_unknown = OutputReport::new(
            request(),
            OutputObservation::new(
                Some(source_format()),
                Some(PlaybackRate::NORMAL),
                Some(Volume::UNITY),
                Some(1),
                FormatConversionStatus::NotApplied,
                ResamplingStatus::NotApplied,
                ChannelMappingStatus::Identity,
                Some(false),
                None,
            ),
        );

        assert_eq!(
            dsp_unknown.bit_perfect_reason(),
            BitPerfectReason::MissingDspEvidence
        );
        assert_eq!(
            crossfade_unknown.bit_perfect_reason(),
            BitPerfectReason::MissingCrossfadeEvidence
        );
    }
}
