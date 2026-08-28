//! Platform-neutral presentation clock for rendered audio.
//!
//! The clock advances only from rendered output frames supplied by the audio
//! boundary.  It never consults a UI timer or wall clock.  Position arithmetic
//! uses integer fixed-point playback rates so contract tests are reproducible
//! across platforms.

use std::{error::Error, fmt};

/// A positive playback rate represented in millionths of normal speed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PlaybackRate(u32);

impl PlaybackRate {
    /// Normal 1.0x playback.
    pub const NORMAL: Self = Self(1_000_000);

    /// Creates a rate from millionths of normal speed.
    ///
    /// # Errors
    ///
    /// Returns [`ClockError::InvalidPlaybackRate`] for zero.
    pub const fn from_millionths(value: u32) -> Result<Self, ClockError> {
        if value == 0 {
            return Err(ClockError::InvalidPlaybackRate);
        }
        Ok(Self(value))
    }

    /// Returns the fixed-point millionths value.
    #[must_use]
    pub const fn as_millionths(self) -> u32 {
        self.0
    }
}

impl Default for PlaybackRate {
    fn default() -> Self {
        Self::NORMAL
    }
}

/// Why a presentation timeline became discontinuous.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DiscontinuityReason {
    /// A worker-side seek took effect.
    Seek,
    /// Output consumption stopped at pause.
    Pause,
    /// Output consumption restarted after pause.
    Resume,
    /// The active session stopped.
    Stop,
    /// A different source became active.
    SourceTransition,
    /// An output path was restarted.
    OutputRestart,
    /// An underrun recovery path became active.
    UnderrunRecovery,
    /// Playback rate changed.
    PlaybackRateChanged,
    /// Output sample rate changed.
    SampleRateChanged,
}

/// Errors from checked presentation-clock arithmetic or configuration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClockError {
    /// A sample rate of zero cannot define a timeline.
    InvalidSampleRate,
    /// A playback rate of zero cannot advance media time.
    InvalidPlaybackRate,
    /// The epoch cannot be incremented without wrapping.
    EpochExhausted,
    /// Rendered-frame accounting would wrap.
    FrameOverflow,
    /// Media-position arithmetic would wrap.
    PositionOverflow,
}

impl fmt::Display for ClockError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidSampleRate => formatter.write_str("output sample rate must be non-zero"),
            Self::InvalidPlaybackRate => {
                formatter.write_str("playback rate must be greater than zero")
            }
            Self::EpochExhausted => formatter.write_str("presentation clock epoch is exhausted"),
            Self::FrameOverflow => formatter.write_str("rendered frame count would overflow"),
            Self::PositionOverflow => formatter.write_str("media position would overflow"),
        }
    }
}

impl Error for ClockError {}

/// The single presentation timeline consumed by UI, lyrics, controls, and
/// playback-history finalization.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PresentationClock {
    clock_epoch: u64,
    rendered_frames: u64,
    media_position_us: u64,
    playback_rate: PlaybackRate,
    output_sample_rate: u32,
    estimated_output_latency_frames: u64,
    discontinuity: bool,
    discontinuity_reason: Option<DiscontinuityReason>,
    timeline_base_position_us: u64,
}

impl PresentationClock {
    /// Creates a normal-speed clock at position zero.
    ///
    /// # Errors
    ///
    /// Returns [`ClockError::InvalidSampleRate`] for a zero sample rate.
    pub fn new(output_sample_rate: u32) -> Result<Self, ClockError> {
        Self::with_latency(output_sample_rate, 0)
    }

    /// Creates a clock with an initial output-latency estimate.
    ///
    /// # Errors
    ///
    /// Returns [`ClockError::InvalidSampleRate`] for a zero sample rate.
    pub fn with_latency(
        output_sample_rate: u32,
        estimated_output_latency_frames: u64,
    ) -> Result<Self, ClockError> {
        if output_sample_rate == 0 {
            return Err(ClockError::InvalidSampleRate);
        }

        Ok(Self {
            clock_epoch: 0,
            rendered_frames: 0,
            media_position_us: 0,
            playback_rate: PlaybackRate::NORMAL,
            output_sample_rate,
            estimated_output_latency_frames,
            discontinuity: false,
            discontinuity_reason: None,
            timeline_base_position_us: 0,
        })
    }

    /// Returns the monotonically increasing discontinuity epoch.
    #[must_use]
    pub const fn clock_epoch(self) -> u64 {
        self.clock_epoch
    }

    /// Returns output frames rendered since the latest discontinuity.
    #[must_use]
    pub const fn rendered_frames(self) -> u64 {
        self.rendered_frames
    }

    /// Returns the media position represented by the rendered output.
    #[must_use]
    pub const fn media_position_us(self) -> u64 {
        self.media_position_us
    }

    /// Returns the fixed-point playback rate.
    #[must_use]
    pub const fn playback_rate(self) -> PlaybackRate {
        self.playback_rate
    }

    /// Returns the observed output sample rate.
    #[must_use]
    pub const fn output_sample_rate(self) -> u32 {
        self.output_sample_rate
    }

    /// Returns the latest output-latency estimate in frames.
    #[must_use]
    pub const fn estimated_output_latency_frames(self) -> u64 {
        self.estimated_output_latency_frames
    }

    /// Returns whether a consumer still needs to observe a discontinuity.
    #[must_use]
    pub const fn is_discontinuous(self) -> bool {
        self.discontinuity
    }

    /// Returns the reason for the pending discontinuity, if any.
    #[must_use]
    pub const fn discontinuity_reason(self) -> Option<DiscontinuityReason> {
        self.discontinuity_reason
    }

    /// Returns a copy suitable for a bounded latest-value snapshot.
    #[must_use]
    pub const fn snapshot(self) -> Self {
        self
    }

    /// Adds rendered output frames and advances media position deterministically.
    ///
    /// The caller is responsible for invoking this only for frames actually
    /// consumed by the output boundary.  Zero frames is a no-op.
    ///
    /// # Errors
    ///
    /// Returns a checked overflow error and leaves the clock unchanged.
    pub fn advance_rendered_frames(&mut self, frames: u64) -> Result<(), ClockError> {
        if frames == 0 {
            return Ok(());
        }

        let rendered_frames = self
            .rendered_frames
            .checked_add(frames)
            .ok_or(ClockError::FrameOverflow)?;
        let media_position_us = self.position_for_frames(rendered_frames)?;
        self.rendered_frames = rendered_frames;
        self.media_position_us = media_position_us;
        Ok(())
    }

    /// Applies a worker-side seek and starts a new presentation epoch.
    ///
    /// # Errors
    ///
    /// Returns [`ClockError::EpochExhausted`] without mutating the clock when
    /// the epoch cannot be advanced.
    pub fn seek(&mut self, position_us: u64) -> Result<(), ClockError> {
        self.begin_epoch(DiscontinuityReason::Seek, position_us)
    }

    /// Marks the pause boundary while retaining the current media position.
    ///
    /// # Errors
    ///
    /// Returns [`ClockError::EpochExhausted`] without mutating the clock when
    /// the epoch cannot be advanced.
    pub fn pause_boundary(&mut self) -> Result<(), ClockError> {
        self.begin_epoch(DiscontinuityReason::Pause, self.media_position_us)
    }

    /// Marks the resume boundary while retaining the current media position.
    ///
    /// # Errors
    ///
    /// Returns [`ClockError::EpochExhausted`] without mutating the clock when
    /// the epoch cannot be advanced.
    pub fn resume_boundary(&mut self) -> Result<(), ClockError> {
        self.begin_epoch(DiscontinuityReason::Resume, self.media_position_us)
    }

    /// Marks a source transition and selects its initial media position.
    ///
    /// # Errors
    ///
    /// Returns [`ClockError::EpochExhausted`] without mutating the clock when
    /// the epoch cannot be advanced.
    pub fn source_transition(&mut self, position_us: u64) -> Result<(), ClockError> {
        self.begin_epoch(DiscontinuityReason::SourceTransition, position_us)
    }

    /// Marks an output restart without changing media position.
    ///
    /// # Errors
    ///
    /// Returns [`ClockError::EpochExhausted`] without mutating the clock when
    /// the epoch cannot be advanced.
    pub fn output_restart(&mut self) -> Result<(), ClockError> {
        self.begin_epoch(DiscontinuityReason::OutputRestart, self.media_position_us)
    }

    /// Marks an underrun recovery boundary without changing media position.
    ///
    /// # Errors
    ///
    /// Returns [`ClockError::EpochExhausted`] without mutating the clock when
    /// the epoch cannot be advanced.
    pub fn underrun_recovery(&mut self) -> Result<(), ClockError> {
        self.begin_epoch(
            DiscontinuityReason::UnderrunRecovery,
            self.media_position_us,
        )
    }

    /// Resets the media position at the stop boundary.
    ///
    /// # Errors
    ///
    /// Returns [`ClockError::EpochExhausted`] without mutating the clock when
    /// the epoch cannot be advanced.
    pub fn stop_boundary(&mut self) -> Result<(), ClockError> {
        self.begin_epoch(DiscontinuityReason::Stop, 0)
    }

    /// Changes playback rate and starts a new timeline epoch.
    ///
    /// # Errors
    ///
    /// Returns [`ClockError::EpochExhausted`] or a rate validation error while
    /// preserving the old configuration.
    pub fn set_playback_rate(&mut self, playback_rate: PlaybackRate) -> Result<(), ClockError> {
        if playback_rate == self.playback_rate {
            return Ok(());
        }
        let position_us = self.media_position_us;
        self.begin_epoch(DiscontinuityReason::PlaybackRateChanged, position_us)?;
        self.playback_rate = playback_rate;
        Ok(())
    }

    /// Changes the observed output sample rate and starts a new epoch.
    ///
    /// # Errors
    ///
    /// Returns [`ClockError::InvalidSampleRate`] or [`ClockError::EpochExhausted`]
    /// while preserving the old configuration.
    pub fn set_output_sample_rate(&mut self, output_sample_rate: u32) -> Result<(), ClockError> {
        if output_sample_rate == 0 {
            return Err(ClockError::InvalidSampleRate);
        }
        if output_sample_rate == self.output_sample_rate {
            return Ok(());
        }
        let position_us = self.media_position_us;
        self.begin_epoch(DiscontinuityReason::SampleRateChanged, position_us)?;
        self.output_sample_rate = output_sample_rate;
        Ok(())
    }

    /// Updates output latency metadata without changing the media timeline.
    pub const fn set_output_latency_frames(&mut self, frames: u64) {
        self.estimated_output_latency_frames = frames;
    }

    /// Acknowledges the currently exposed discontinuity.
    pub fn acknowledge_discontinuity(&mut self) {
        self.discontinuity = false;
        self.discontinuity_reason = None;
    }

    fn begin_epoch(
        &mut self,
        reason: DiscontinuityReason,
        position_us: u64,
    ) -> Result<(), ClockError> {
        let epoch = self
            .clock_epoch
            .checked_add(1)
            .ok_or(ClockError::EpochExhausted)?;
        self.clock_epoch = epoch;
        self.rendered_frames = 0;
        self.media_position_us = position_us;
        self.timeline_base_position_us = position_us;
        self.discontinuity = true;
        self.discontinuity_reason = Some(reason);
        Ok(())
    }

    fn position_for_frames(&self, frames: u64) -> Result<u64, ClockError> {
        let delta_us = u128::from(frames)
            .checked_mul(u128::from(self.playback_rate.as_millionths()))
            .ok_or(ClockError::PositionOverflow)?
            / u128::from(self.output_sample_rate);
        let position = u128::from(self.timeline_base_position_us)
            .checked_add(delta_us)
            .ok_or(ClockError::PositionOverflow)?;
        u64::try_from(position).map_err(|_| ClockError::PositionOverflow)
    }
}
