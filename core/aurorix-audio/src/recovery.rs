//! Non-realtime recovery coordination for the local audio pipeline.
//!
//! Realtime code only publishes bounded signals. This coordinator is consumed
//! by a worker or control-plane thread; it never waits for the callback and it
//! keeps the recovery state and its pending underrun summary bounded.

use std::{error::Error, fmt};

use crate::seek::BufferGeneration;

/// A safe upper-level state for the recovery coordinator.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RecoveryState {
    /// No recovery work is pending.
    Ready,
    /// The output path is waiting for enough prepared data.
    Buffering,
    /// An underrun recovery is rebuilding the active data path.
    Recovering,
    /// The output path is being reopened after a restart request.
    OutputRestarting,
    /// The decoder/output graph is being rebuilt.
    GraphRebuilding,
}

/// The reason for a recovery transition.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RecoveryCause {
    /// The callback observed one or more missing output frames.
    Underrun,
    /// The output endpoint needs to be reopened.
    OutputRestart,
    /// The observed output sample rate changed.
    SampleRateChange,
    /// The worker rebuilt the decoder/DSP graph.
    GraphRebuild,
    /// The coordinator entered a buffering boundary.
    Buffering,
}

/// A bounded summary of underruns merged before worker-side recovery.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct UnderrunSummary {
    event_count: u64,
    total_missing_frames: u64,
    largest_missing_frames: u32,
}

impl UnderrunSummary {
    /// Creates a summary for one positive missing-frame observation.
    #[must_use]
    pub const fn first(missing_frames: u32) -> Self {
        Self {
            event_count: 1,
            total_missing_frames: missing_frames as u64,
            largest_missing_frames: missing_frames,
        }
    }

    /// Returns the number of merged callback observations.
    #[must_use]
    pub const fn event_count(self) -> u64 {
        self.event_count
    }

    /// Returns the saturating sum of missing frames.
    #[must_use]
    pub const fn total_missing_frames(self) -> u64 {
        self.total_missing_frames
    }

    /// Returns the largest missing-frame observation in this summary.
    #[must_use]
    pub const fn largest_missing_frames(self) -> u32 {
        self.largest_missing_frames
    }

    fn merge(&mut self, missing_frames: u32) {
        self.event_count = self.event_count.saturating_add(1);
        self.total_missing_frames = self
            .total_missing_frames
            .saturating_add(u64::from(missing_frames));
        self.largest_missing_frames = self.largest_missing_frames.max(missing_frames);
    }
}

/// One committed, observable recovery boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RecoveryTransition {
    clock_epoch: u64,
    buffer_generation: BufferGeneration,
    cause: RecoveryCause,
    discontinuity: bool,
    output_sample_rate_hz: u32,
    underrun: Option<UnderrunSummary>,
}

impl RecoveryTransition {
    /// Returns the new clock epoch to be applied to the presentation clock.
    #[must_use]
    pub const fn clock_epoch(self) -> u64 {
        self.clock_epoch
    }

    /// Returns the new decoded-buffer generation.
    #[must_use]
    pub const fn buffer_generation(self) -> BufferGeneration {
        self.buffer_generation
    }

    /// Returns why the boundary was created.
    #[must_use]
    pub const fn cause(self) -> RecoveryCause {
        self.cause
    }

    /// Every committed recovery boundary is visible to downstream consumers.
    #[must_use]
    pub const fn is_discontinuity(self) -> bool {
        self.discontinuity
    }

    /// Returns the currently observed output sample rate.
    #[must_use]
    pub const fn output_sample_rate_hz(self) -> u32 {
        self.output_sample_rate_hz
    }

    /// Returns the merged underrun evidence for an underrun transition.
    #[must_use]
    pub const fn underrun(self) -> Option<UnderrunSummary> {
        self.underrun
    }
}

/// Errors from checked recovery state transitions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecoveryError {
    /// A zero sample rate cannot describe an output timeline.
    InvalidSampleRate,
    /// A zero missing-frame signal is not an underrun.
    InvalidUnderrunFrames,
    /// No callback underrun is waiting for worker-side recovery.
    NoPendingUnderrun,
    /// The clock epoch cannot advance without wrapping.
    ClockEpochExhausted,
    /// The decoded-buffer generation cannot advance without wrapping.
    BufferGenerationExhausted,
    /// The coordinator could not produce the transition retained for an
    /// already-active state.
    StateInvariant,
    /// A sample-rate request did not change the observed rate.
    SampleRateUnchanged,
}

impl fmt::Display for RecoveryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidSampleRate => formatter.write_str("output sample rate must be non-zero"),
            Self::InvalidUnderrunFrames => {
                formatter.write_str("an underrun must contain at least one missing frame")
            }
            Self::NoPendingUnderrun => formatter.write_str("no underrun is pending recovery"),
            Self::ClockEpochExhausted => formatter.write_str("recovery clock epoch is exhausted"),
            Self::BufferGenerationExhausted => {
                formatter.write_str("recovery buffer generation is exhausted")
            }
            Self::StateInvariant => formatter.write_str("recovery state invariant is violated"),
            Self::SampleRateUnchanged => formatter.write_str("output sample rate is unchanged"),
        }
    }
}

impl Error for RecoveryError {}

/// Worker-side recovery coordinator.
#[derive(Debug)]
pub struct RecoveryCoordinator {
    state: RecoveryState,
    clock_epoch: u64,
    buffer_generation: BufferGeneration,
    output_sample_rate_hz: u32,
    pending_underrun: Option<UnderrunSummary>,
    last_transition: Option<RecoveryTransition>,
}

impl RecoveryCoordinator {
    /// Creates a coordinator at epoch and buffer generation zero.
    ///
    /// # Errors
    ///
    /// Returns [`RecoveryError::InvalidSampleRate`] for a zero rate.
    pub fn new(output_sample_rate_hz: u32) -> Result<Self, RecoveryError> {
        if output_sample_rate_hz == 0 {
            return Err(RecoveryError::InvalidSampleRate);
        }
        Ok(Self {
            state: RecoveryState::Ready,
            clock_epoch: 0,
            buffer_generation: BufferGeneration::INITIAL,
            output_sample_rate_hz,
            pending_underrun: None,
            last_transition: None,
        })
    }

    /// Returns the current recovery state.
    #[must_use]
    pub const fn state(&self) -> RecoveryState {
        self.state
    }

    /// Returns the current clock epoch.
    #[must_use]
    pub const fn clock_epoch(&self) -> u64 {
        self.clock_epoch
    }

    /// Returns the generation accepted by a new decoded buffer.
    #[must_use]
    pub const fn buffer_generation(&self) -> BufferGeneration {
        self.buffer_generation
    }

    /// Returns the currently observed output sample rate.
    #[must_use]
    pub const fn output_sample_rate_hz(&self) -> u32 {
        self.output_sample_rate_hz
    }

    /// Returns a copy of the pending merged underrun, if any.
    #[must_use]
    pub const fn pending_underrun(&self) -> Option<UnderrunSummary> {
        self.pending_underrun
    }

    /// Returns the latest committed discontinuity boundary.
    #[must_use]
    pub const fn last_transition(&self) -> Option<RecoveryTransition> {
        self.last_transition
    }

    /// Merges one callback-side underrun observation.
    ///
    /// This method is intended for the non-realtime coordinator after it has
    /// consumed the bounded signal cell. Repeated observations are merged and
    /// do not create repeated epochs before recovery is actually committed.
    ///
    /// # Errors
    ///
    /// Returns [`RecoveryError::InvalidUnderrunFrames`] for zero.
    pub fn record_underrun(&mut self, missing_frames: u32) -> Result<(), RecoveryError> {
        if missing_frames == 0 {
            return Err(RecoveryError::InvalidUnderrunFrames);
        }
        if let Some(summary) = &mut self.pending_underrun {
            summary.merge(missing_frames);
        } else {
            self.pending_underrun = Some(UnderrunSummary::first(missing_frames));
        }
        Ok(())
    }

    /// Enters buffering once and exposes a discontinuity boundary.
    ///
    /// Repeated calls while already buffering return the same boundary and do
    /// not advance either counter again.
    ///
    /// # Errors
    ///
    /// Returns a checked counter error if a new boundary cannot be represented.
    pub fn enter_buffering(&mut self) -> Result<RecoveryTransition, RecoveryError> {
        if self.state == RecoveryState::Buffering
            && self
                .last_transition
                .is_some_and(|transition| transition.cause() == RecoveryCause::Buffering)
        {
            return self.last_transition.ok_or(RecoveryError::StateInvariant);
        }
        self.commit_transition(RecoveryCause::Buffering, RecoveryState::Buffering, None)
    }

    /// Commits one recovery boundary for all underruns merged so far.
    ///
    /// # Errors
    ///
    /// Returns [`RecoveryError::NoPendingUnderrun`] when no signal is waiting,
    /// or a checked counter error when a new boundary cannot be represented.
    pub fn recover_underrun(&mut self) -> Result<RecoveryTransition, RecoveryError> {
        let underrun = self
            .pending_underrun
            .ok_or(RecoveryError::NoPendingUnderrun)?;
        let transition = self.commit_transition(
            RecoveryCause::Underrun,
            RecoveryState::Recovering,
            Some(underrun),
        );
        if transition.is_ok() {
            self.pending_underrun = None;
        }
        transition
    }

    /// Commits an output restart boundary without waiting for the output host.
    ///
    /// # Errors
    ///
    /// Returns a checked counter error if the transition cannot be represented.
    pub fn restart_output(&mut self) -> Result<RecoveryTransition, RecoveryError> {
        self.commit_transition(
            RecoveryCause::OutputRestart,
            RecoveryState::OutputRestarting,
            None,
        )
    }

    /// Commits a changed output sample-rate boundary.
    ///
    /// # Errors
    ///
    /// Returns [`RecoveryError::SampleRateUnchanged`] when the rate is equal to
    /// the observed value, or a validation/counter error otherwise.
    pub fn change_sample_rate(
        &mut self,
        output_sample_rate_hz: u32,
    ) -> Result<RecoveryTransition, RecoveryError> {
        if output_sample_rate_hz == 0 {
            return Err(RecoveryError::InvalidSampleRate);
        }
        if output_sample_rate_hz == self.output_sample_rate_hz {
            return Err(RecoveryError::SampleRateUnchanged);
        }
        let transition = self.commit_transition(
            RecoveryCause::SampleRateChange,
            RecoveryState::Buffering,
            None,
        )?;
        self.output_sample_rate_hz = output_sample_rate_hz;
        let transition = RecoveryTransition {
            output_sample_rate_hz,
            ..transition
        };
        self.last_transition = Some(transition);
        Ok(transition)
    }

    /// Commits a graph rebuild boundary.
    ///
    /// # Errors
    ///
    /// Returns a checked counter error if the transition cannot be represented.
    pub fn rebuild_graph(&mut self) -> Result<RecoveryTransition, RecoveryError> {
        self.commit_transition(
            RecoveryCause::GraphRebuild,
            RecoveryState::GraphRebuilding,
            None,
        )
    }

    /// Marks the worker path ready after the current recovery work is complete.
    ///
    /// This does not advance the timeline. The discontinuity is already carried
    /// by the transition returned when the recovery boundary was committed.
    pub fn mark_ready(&mut self) {
        self.state = RecoveryState::Ready;
    }

    fn commit_transition(
        &mut self,
        cause: RecoveryCause,
        state: RecoveryState,
        underrun: Option<UnderrunSummary>,
    ) -> Result<RecoveryTransition, RecoveryError> {
        let clock_epoch = self
            .clock_epoch
            .checked_add(1)
            .ok_or(RecoveryError::ClockEpochExhausted)?;
        let generation_value = self
            .buffer_generation
            .value()
            .checked_add(1)
            .ok_or(RecoveryError::BufferGenerationExhausted)?;
        let transition = RecoveryTransition {
            clock_epoch,
            buffer_generation: BufferGeneration::new(generation_value),
            cause,
            discontinuity: true,
            output_sample_rate_hz: self.output_sample_rate_hz,
            underrun,
        };
        self.clock_epoch = clock_epoch;
        self.buffer_generation = transition.buffer_generation;
        self.state = state;
        self.last_transition = Some(transition);
        Ok(transition)
    }
}

#[cfg(test)]
mod tests {
    use super::{RecoveryCause, RecoveryCoordinator, RecoveryError, RecoveryState};

    #[test]
    fn repeated_underruns_converge_to_one_recovery_boundary() {
        let mut coordinator = RecoveryCoordinator::new(48_000).expect("valid rate");
        coordinator.record_underrun(32).expect("positive signal");
        coordinator.record_underrun(64).expect("positive signal");
        coordinator.record_underrun(16).expect("positive signal");

        let summary = coordinator.pending_underrun().expect("merged signal");
        assert_eq!(summary.event_count(), 3);
        assert_eq!(summary.total_missing_frames(), 112);
        assert_eq!(summary.largest_missing_frames(), 64);

        let transition = coordinator.recover_underrun().expect("recovery boundary");
        assert_eq!(transition.cause(), RecoveryCause::Underrun);
        assert_eq!(transition.clock_epoch(), 1);
        assert_eq!(transition.buffer_generation().value(), 1);
        assert!(transition.is_discontinuity());
        assert_eq!(transition.underrun(), Some(summary));
        assert_eq!(coordinator.state(), RecoveryState::Recovering);
        assert_eq!(
            coordinator.recover_underrun(),
            Err(RecoveryError::NoPendingUnderrun)
        );
    }

    #[test]
    fn buffering_is_idempotent_and_restart_advances_both_fences() {
        let mut coordinator = RecoveryCoordinator::new(44_100).expect("valid rate");
        let first = coordinator.enter_buffering().expect("buffering boundary");
        let repeated = coordinator
            .enter_buffering()
            .expect("same buffering boundary");
        assert_eq!(first, repeated);
        assert_eq!(coordinator.clock_epoch(), 1);
        assert_eq!(coordinator.buffer_generation().value(), 1);

        let restart = coordinator.restart_output().expect("restart boundary");
        assert_eq!(restart.cause(), RecoveryCause::OutputRestart);
        assert_eq!(restart.clock_epoch(), 2);
        assert_eq!(restart.buffer_generation().value(), 2);
        assert!(restart.is_discontinuity());
    }

    #[test]
    fn rate_change_and_graph_rebuild_publish_distinct_boundaries() {
        let mut coordinator = RecoveryCoordinator::new(48_000).expect("valid rate");
        let rate = coordinator
            .change_sample_rate(96_000)
            .expect("changed rate");
        assert_eq!(rate.cause(), RecoveryCause::SampleRateChange);
        assert_eq!(rate.output_sample_rate_hz(), 96_000);
        assert_eq!(coordinator.output_sample_rate_hz(), 96_000);

        let graph = coordinator.rebuild_graph().expect("graph boundary");
        assert_eq!(graph.cause(), RecoveryCause::GraphRebuild);
        assert_eq!(graph.clock_epoch(), 2);
        assert_eq!(graph.buffer_generation().value(), 2);
        coordinator.mark_ready();
        assert_eq!(coordinator.state(), RecoveryState::Ready);
    }

    #[test]
    fn invalid_signals_do_not_change_recovery_counters() {
        let mut coordinator = RecoveryCoordinator::new(48_000).expect("valid rate");
        assert_eq!(
            coordinator.record_underrun(0),
            Err(RecoveryError::InvalidUnderrunFrames)
        );
        assert_eq!(
            coordinator.change_sample_rate(48_000),
            Err(RecoveryError::SampleRateUnchanged)
        );
        assert_eq!(coordinator.clock_epoch(), 0);
        assert_eq!(coordinator.buffer_generation().value(), 0);
    }
}
