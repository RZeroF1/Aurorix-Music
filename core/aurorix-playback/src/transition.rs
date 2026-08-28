//! Offline playback transition policy.
//!
//! This module contains control-plane decisions only. It does not open a
//! source, own a decoder, or run on the realtime callback. A transition is
//! gapless only when the next item is prepared, has enough PCM, and its codec
//! delay/padding facts are known. Crossfade is an explicit mixed-DSP path and
//! can never report strict bit-perfect eligibility.

use std::{error::Error, fmt};

/// The release-one local-file prebuffer target.
pub const DEFAULT_LOCAL_PREBUFFER_MS: u32 = 100;

/// A bounded prebuffer target expressed in output frames.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PrebufferTarget {
    sample_rate_hz: u32,
    target_frames: u64,
}

impl PrebufferTarget {
    /// Creates a target from a sample rate and duration.
    ///
    /// The frame count rounds up so the target is never shorter than the
    /// requested duration.
    ///
    /// # Errors
    ///
    /// Returns `TransitionError::InvalidSampleRate` for zero and
    /// `TransitionError::FrameCountOverflow` when the calculation cannot be
    /// represented.
    pub fn from_milliseconds(
        sample_rate_hz: u32,
        milliseconds: u32,
    ) -> Result<Self, TransitionError> {
        if sample_rate_hz == 0 {
            return Err(TransitionError::InvalidSampleRate);
        }
        let numerator = u64::from(sample_rate_hz)
            .checked_mul(u64::from(milliseconds))
            .ok_or(TransitionError::FrameCountOverflow)?;
        let target_frames = numerator
            .checked_add(999)
            .ok_or(TransitionError::FrameCountOverflow)?
            / 1_000;
        Ok(Self {
            sample_rate_hz,
            target_frames,
        })
    }

    /// Returns the output sample rate used for this target.
    #[must_use]
    pub const fn sample_rate_hz(self) -> u32 {
        self.sample_rate_hz
    }

    /// Returns the minimum number of buffered frames required.
    #[must_use]
    pub const fn target_frames(self) -> u64 {
        self.target_frames
    }
}

impl Default for PrebufferTarget {
    fn default() -> Self {
        Self::from_milliseconds(48_000, DEFAULT_LOCAL_PREBUFFER_MS)
            .expect("the default prebuffer target is valid")
    }
}

/// A worker-updated bounded prebuffer observation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PrebufferState {
    buffered_frames: u64,
    target: PrebufferTarget,
}

impl PrebufferState {
    /// Creates an empty observation for a target.
    #[must_use]
    pub const fn new(target: PrebufferTarget) -> Self {
        Self {
            buffered_frames: 0,
            target,
        }
    }

    /// Returns the configured target.
    #[must_use]
    pub const fn target(self) -> PrebufferTarget {
        self.target
    }

    /// Returns the currently buffered frame count.
    #[must_use]
    pub const fn buffered_frames(self) -> u64 {
        self.buffered_frames
    }

    /// Replaces the current bounded observation.
    pub const fn set_buffered_frames(&mut self, buffered_frames: u64) {
        self.buffered_frames = buffered_frames;
    }

    /// Adds decoded frames, saturating at the representable maximum.
    pub const fn add_buffered_frames(&mut self, frames: u64) {
        self.buffered_frames = self.buffered_frames.saturating_add(frames);
    }

    /// Consumes rendered frames, saturating at zero.
    pub const fn consume_frames(&mut self, frames: u64) {
        self.buffered_frames = self.buffered_frames.saturating_sub(frames);
    }

    /// Reports whether the target has been reached.
    #[must_use]
    pub const fn is_ready(self) -> bool {
        self.buffered_frames >= self.target.target_frames()
    }

    /// Returns a normalized fill ratio in millionths.
    #[must_use]
    pub fn fill_ratio_millionths(self) -> u32 {
        if self.target.target_frames() == 0 {
            return 1_000_000;
        }
        let numerator = self.buffered_frames.saturating_mul(1_000_000);
        let ratio = numerator / self.target.target_frames();
        if ratio > 1_000_000 {
            1_000_000
        } else {
            u32::try_from(ratio).unwrap_or(1_000_000)
        }
    }
}

/// Codec delay and end padding needed to decide whether a transition is
/// genuinely gapless.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CodecDelayPadding {
    decoder_delay_frames: u64,
    encoder_padding_frames: u64,
    known: bool,
}

impl CodecDelayPadding {
    /// Creates known delay and padding facts.
    #[must_use]
    pub const fn known(decoder_delay_frames: u64, encoder_padding_frames: u64) -> Self {
        Self {
            decoder_delay_frames,
            encoder_padding_frames,
            known: true,
        }
    }

    /// Creates an explicitly unknown delay/padding value.
    #[must_use]
    pub const fn unknown() -> Self {
        Self {
            decoder_delay_frames: 0,
            encoder_padding_frames: 0,
            known: false,
        }
    }

    /// Returns whether both delay and padding are known.
    #[must_use]
    pub const fn is_known(self) -> bool {
        self.known
    }

    /// Returns decoder priming delay in frames.
    #[must_use]
    pub const fn decoder_delay_frames(self) -> u64 {
        self.decoder_delay_frames
    }

    /// Returns encoder end padding in frames.
    #[must_use]
    pub const fn encoder_padding_frames(self) -> u64 {
        self.encoder_padding_frames
    }
}

/// A prepared next decoder observation held by the worker/coordinator.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PreparedNext {
    item_index: usize,
    buffer_generation: u64,
    prebuffer: PrebufferState,
    delay_padding: CodecDelayPadding,
}

impl PreparedNext {
    /// Creates a next-item observation without runtime handles or paths.
    #[must_use]
    pub const fn new(
        item_index: usize,
        buffer_generation: u64,
        prebuffer: PrebufferState,
        delay_padding: CodecDelayPadding,
    ) -> Self {
        Self {
            item_index,
            buffer_generation,
            prebuffer,
            delay_padding,
        }
    }

    /// Returns the logical queue index of the prepared item.
    #[must_use]
    pub const fn item_index(self) -> usize {
        self.item_index
    }

    /// Returns the generation for which this preparation is valid.
    #[must_use]
    pub const fn buffer_generation(self) -> u64 {
        self.buffer_generation
    }

    /// Returns the prebuffer observation.
    #[must_use]
    pub const fn prebuffer(self) -> PrebufferState {
        self.prebuffer
    }

    /// Returns the delay/padding facts.
    #[must_use]
    pub const fn delay_padding(self) -> CodecDelayPadding {
        self.delay_padding
    }

    /// Reports whether this prepared item satisfies the gapless data contract.
    #[must_use]
    pub const fn gapless_ready(self) -> bool {
        self.prebuffer.is_ready() && self.delay_padding.is_known()
    }
}

/// Explicit crossfade configuration.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CrossfadePolicy {
    enabled: bool,
    duration_ms: u32,
}

impl CrossfadePolicy {
    /// Crossfade disabled; a transition may be gapless when its facts qualify.
    pub const DISABLED: Self = Self {
        enabled: false,
        duration_ms: 0,
    };

    /// Creates a crossfade policy.
    #[must_use]
    pub const fn new(enabled: bool, duration_ms: u32) -> Self {
        Self {
            enabled: enabled && duration_ms != 0,
            duration_ms: if enabled { duration_ms } else { 0 },
        }
    }

    /// Returns whether crossfade is enabled.
    #[must_use]
    pub const fn enabled(self) -> bool {
        self.enabled
    }

    /// Returns the configured duration.
    #[must_use]
    pub const fn duration_ms(self) -> u32 {
        self.duration_ms
    }
}

impl Default for CrossfadePolicy {
    fn default() -> Self {
        Self::DISABLED
    }
}

/// The decision made for a candidate next item.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TransitionDecision {
    /// The next item may take over without a mixed DSP operation.
    Gapless {
        /// The next item's logical queue index.
        item_index: usize,
    },
    /// The next item may overlap through the explicit crossfade mixer.
    Crossfade {
        /// The next item's logical queue index.
        item_index: usize,
        /// Crossfade duration in milliseconds.
        duration_ms: u32,
    },
    /// The next item is not ready; the coordinator should buffer.
    Buffering,
    /// No transition is eligible.
    None,
}

impl TransitionDecision {
    /// Returns the selected next item, if one is eligible.
    #[must_use]
    pub const fn item_index(self) -> Option<usize> {
        match self {
            Self::Gapless { item_index } | Self::Crossfade { item_index, .. } => Some(item_index),
            Self::Buffering | Self::None => None,
        }
    }

    /// Returns whether this decision uses mixed DSP.
    #[must_use]
    pub const fn is_mixed_dsp(self) -> bool {
        matches!(self, Self::Crossfade { .. })
    }

    /// Strict bit-perfect output is never eligible for a crossfade.
    #[must_use]
    pub const fn bit_perfect_eligible(self) -> bool {
        matches!(self, Self::Gapless { .. })
    }
}

/// Evaluates gapless and crossfade policy at a control-plane boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TransitionPlanner {
    crossfade: CrossfadePolicy,
}

impl TransitionPlanner {
    /// Creates a planner with the supplied explicit crossfade policy.
    #[must_use]
    pub const fn new(crossfade: CrossfadePolicy) -> Self {
        Self { crossfade }
    }

    /// Returns the configured crossfade policy.
    #[must_use]
    pub const fn crossfade(self) -> CrossfadePolicy {
        self.crossfade
    }

    /// Chooses the transition when the prepared item belongs to the active
    /// generation.
    #[must_use]
    pub const fn decide(
        self,
        current_generation: u64,
        prepared: Option<PreparedNext>,
    ) -> TransitionDecision {
        let Some(prepared) = prepared else {
            return TransitionDecision::None;
        };
        if prepared.buffer_generation() != current_generation {
            return TransitionDecision::Buffering;
        }
        if self.crossfade.enabled() {
            if prepared.prebuffer().is_ready() {
                return TransitionDecision::Crossfade {
                    item_index: prepared.item_index(),
                    duration_ms: self.crossfade.duration_ms(),
                };
            }
            return TransitionDecision::Buffering;
        }
        if prepared.gapless_ready() {
            TransitionDecision::Gapless {
                item_index: prepared.item_index(),
            }
        } else {
            TransitionDecision::Buffering
        }
    }
}

impl Default for TransitionPlanner {
    fn default() -> Self {
        Self::new(CrossfadePolicy::DISABLED)
    }
}

/// A bounded, caller-provided crossfade mixer.
///
/// The method performs no allocation and writes only into the supplied output
/// slice. It uses deterministic linear gains; the presence of this operation
/// is itself sufficient to disable strict bit-perfect eligibility.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CrossfadeMixer {
    channels: usize,
}

impl CrossfadeMixer {
    /// Creates a mixer for an interleaved channel count.
    ///
    /// # Errors
    ///
    /// Returns `TransitionError::InvalidChannelCount` for zero channels.
    pub const fn new(channels: usize) -> Result<Self, TransitionError> {
        if channels == 0 {
            return Err(TransitionError::InvalidChannelCount);
        }
        Ok(Self { channels })
    }

    /// Returns the interleaved channel count.
    #[must_use]
    pub const fn channels(self) -> usize {
        self.channels
    }

    /// Mixes current and next interleaved frames into a caller-owned output.
    ///
    /// The number of frames is limited by all three slices. Missing samples in
    /// either input are treated as zero. The gain progresses linearly from
    /// current-only to next-only and uses no floating-point randomness.
    ///
    /// # Errors
    ///
    /// Returns a typed error when any slice is not frame aligned or output is
    /// too short for the requested frame count.
    #[allow(clippy::cast_precision_loss)]
    pub fn mix_into(
        self,
        current: &[f32],
        next: &[f32],
        output: &mut [f32],
        frames: usize,
    ) -> Result<(), TransitionError> {
        let channels = self.channels;
        if !current.len().is_multiple_of(channels)
            || !next.len().is_multiple_of(channels)
            || !output.len().is_multiple_of(channels)
        {
            return Err(TransitionError::MisalignedSamples);
        }
        let required_samples = frames
            .checked_mul(channels)
            .ok_or(TransitionError::FrameCountOverflow)?;
        if output.len() < required_samples {
            return Err(TransitionError::OutputTooShort {
                required_samples,
                actual_samples: output.len(),
            });
        }
        for frame in 0..frames {
            let progress = if frames <= 1 {
                1.0
            } else {
                frame as f32 / (frames - 1) as f32
            };
            let current_gain = 1.0 - progress;
            let next_gain = progress;
            let start = frame * channels;
            for channel in 0..channels {
                let index = start + channel;
                let current_sample = current.get(index).copied().unwrap_or(0.0);
                let next_sample = next.get(index).copied().unwrap_or(0.0);
                output[index] = current_sample * current_gain + next_sample * next_gain;
            }
        }
        Ok(())
    }
}

/// Errors from transition target construction or bounded mixing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransitionError {
    /// A zero output sample rate was supplied.
    InvalidSampleRate,
    /// A zero channel count was supplied.
    InvalidChannelCount,
    /// A frame/sample calculation overflowed.
    FrameCountOverflow,
    /// An interleaved sample slice was not frame aligned.
    MisalignedSamples,
    /// The output slice cannot hold the requested frame count.
    OutputTooShort {
        /// Required number of interleaved samples.
        required_samples: usize,
        /// Supplied number of interleaved samples.
        actual_samples: usize,
    },
}

impl fmt::Display for TransitionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidSampleRate => {
                formatter.write_str("transition sample rate must be non-zero")
            }
            Self::InvalidChannelCount => {
                formatter.write_str("transition channels must be non-zero")
            }
            Self::FrameCountOverflow => formatter.write_str("transition frame count overflowed"),
            Self::MisalignedSamples => {
                formatter.write_str("interleaved samples are not frame aligned")
            }
            Self::OutputTooShort {
                required_samples,
                actual_samples,
            } => write!(
                formatter,
                "transition output needs {required_samples} samples, got {actual_samples}"
            ),
        }
    }
}

impl Error for TransitionError {}

#[cfg(test)]
mod tests {
    use super::{
        CodecDelayPadding, CrossfadeMixer, CrossfadePolicy, PrebufferState, PrebufferTarget,
        PreparedNext, TransitionDecision, TransitionError, TransitionPlanner,
    };

    #[test]
    fn local_target_rounds_up_and_prebuffer_is_bounded() {
        let target = PrebufferTarget::from_milliseconds(48_000, 100).expect("target is valid");
        assert_eq!(target.target_frames(), 4_800);
        let mut state = PrebufferState::new(target);
        state.add_buffered_frames(4_799);
        assert!(!state.is_ready());
        state.add_buffered_frames(1);
        assert!(state.is_ready());
        assert_eq!(state.fill_ratio_millionths(), 1_000_000);
        state.consume_frames(4_800);
        assert_eq!(state.buffered_frames(), 0);
    }

    #[test]
    fn gapless_requires_prepared_generation_and_known_codec_facts() {
        let target = PrebufferTarget::from_milliseconds(48_000, 100).expect("target is valid");
        let mut prebuffer = PrebufferState::new(target);
        prebuffer.set_buffered_frames(4_800);
        let planner = TransitionPlanner::default();
        let unknown = PreparedNext::new(2, 7, prebuffer, CodecDelayPadding::unknown());
        assert_eq!(
            planner.decide(7, Some(unknown)),
            TransitionDecision::Buffering
        );
        let known = PreparedNext::new(2, 7, prebuffer, CodecDelayPadding::known(529, 1_102));
        assert_eq!(
            planner.decide(7, Some(known)),
            TransitionDecision::Gapless { item_index: 2 }
        );
        assert_eq!(
            planner.decide(8, Some(known)),
            TransitionDecision::Buffering
        );
    }

    #[test]
    fn crossfade_is_explicit_and_never_bit_perfect() {
        let target = PrebufferTarget::from_milliseconds(48_000, 100).expect("target is valid");
        let mut prebuffer = PrebufferState::new(target);
        prebuffer.set_buffered_frames(4_800);
        let prepared = PreparedNext::new(1, 3, prebuffer, CodecDelayPadding::unknown());
        let planner = TransitionPlanner::new(CrossfadePolicy::new(true, 250));
        let decision = planner.decide(3, Some(prepared));
        assert_eq!(
            decision,
            TransitionDecision::Crossfade {
                item_index: 1,
                duration_ms: 250
            }
        );
        assert!(decision.is_mixed_dsp());
        assert!(!decision.bit_perfect_eligible());
    }

    #[test]
    fn crossfade_mixer_is_deterministic_and_bounded() {
        let mixer = CrossfadeMixer::new(2).expect("stereo mixer is valid");
        let mut output = [0.0; 6];
        mixer
            .mix_into(&[1.0, 1.0, 1.0, 1.0], &[0.0; 6], &mut output, 3)
            .expect("mix fits");
        assert!(
            output
                .iter()
                .zip([1.0_f32, 1.0, 0.5, 0.5, 0.0, 0.0])
                .all(|(actual, expected)| actual.to_bits() == expected.to_bits())
        );
        assert_eq!(
            mixer.mix_into(&[1.0], &[0.0], &mut output, 1),
            Err(TransitionError::MisalignedSamples)
        );
    }
}
