//! Idempotent finalization of playback facts from the presentation clock.
//!
//! The builder accepts only portable identity and control-plane metadata. It
//! intentionally has no field for a local path, file handle, stream URL,
//! credential, or Provider lease.

use std::{error::Error, fmt};

use aurorix_model::{
    ids::ReplicatedEntityId,
    media_reference::PortableMediaRef,
    play_fact::{Completion, EndReason, FinalizedPlayFact, PlayFactError},
};

use crate::clock::PresentationClock;

/// A terminal outcome that can finalize one playback fact.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PlaybackOutcome {
    /// The clock reached the normal end of the item.
    Ended,
    /// The item was skipped by the user or queue policy.
    Skipped,
    /// The session was explicitly stopped.
    Stopped,
    /// Source, decoder, or output execution failed.
    Error,
    /// The process ended before the outcome could be confirmed.
    UnknownAfterCrash,
}

impl PlaybackOutcome {
    /// Returns the immutable domain completion classification.
    #[must_use]
    pub const fn completion(self) -> Completion {
        match self {
            Self::Ended => Completion::Completed,
            Self::Skipped | Self::Stopped | Self::Error => Completion::Partial,
            Self::UnknownAfterCrash => Completion::UnknownAfterCrash,
        }
    }

    /// Returns the immutable domain end reason.
    #[must_use]
    pub const fn end_reason(self) -> EndReason {
        match self {
            Self::Ended => EndReason::Ended,
            Self::Skipped => EndReason::Skipped,
            Self::Stopped => EndReason::Stopped,
            Self::Error => EndReason::Error,
            Self::UnknownAfterCrash => EndReason::CrashRecovery,
        }
    }
}

/// Result of an idempotent finalization request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FinalizeResult {
    /// This call created the one finalized fact for the builder.
    Created(FinalizedPlayFact),
    /// The same outcome was already finalized; the existing fact is returned.
    AlreadyFinalized(FinalizedPlayFact),
}

impl FinalizeResult {
    /// Returns the fact regardless of whether this call created it.
    #[must_use]
    pub const fn fact(&self) -> &FinalizedPlayFact {
        match self {
            Self::Created(fact) | Self::AlreadyFinalized(fact) => fact,
        }
    }

    /// Reports whether this call created a new fact.
    #[must_use]
    pub const fn was_created(&self) -> bool {
        matches!(self, Self::Created(_))
    }
}

/// A conflict where a finalized outcome is asked to become a different one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FinalizationConflict {
    /// The outcome that already owns the builder.
    pub existing: PlaybackOutcome,
    /// The new outcome that was rejected.
    pub requested: PlaybackOutcome,
}

/// Errors from playback-fact construction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlaybackFactError {
    /// A second request attempted to change an already-finalized outcome.
    FinalizationConflict(FinalizationConflict),
    /// The immutable model rejected the supplied fact fields.
    Model(PlayFactError),
}

impl fmt::Display for PlaybackFactError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::FinalizationConflict(conflict) => write!(
                formatter,
                "playback fact is already finalized as {:?}; cannot finalize as {:?}",
                conflict.existing, conflict.requested
            ),
            Self::Model(error) => error.fmt(formatter),
        }
    }
}

impl Error for PlaybackFactError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Model(error) => Some(error),
            Self::FinalizationConflict(_) => None,
        }
    }
}

impl From<PlayFactError> for PlaybackFactError {
    fn from(error: PlayFactError) -> Self {
        Self::Model(error)
    }
}

/// The control-plane inputs retained while one item is playing.
#[derive(Debug, Clone)]
pub struct PlaybackFactBuilder {
    fact_id: ReplicatedEntityId,
    media_ref: PortableMediaRef,
    started_at: String,
    duration_ms: u64,
    finalized: Option<(PlaybackOutcome, FinalizedPlayFact)>,
}

impl PlaybackFactBuilder {
    /// Creates a builder from portable identity and validated-by-model time
    /// metadata. The timestamp is checked when the fact is finalized.
    #[must_use]
    pub fn new(
        fact_id: ReplicatedEntityId,
        media_ref: PortableMediaRef,
        started_at: impl Into<String>,
        duration_ms: u64,
    ) -> Self {
        Self {
            fact_id,
            media_ref,
            started_at: started_at.into(),
            duration_ms,
            finalized: None,
        }
    }

    /// Returns the fact ID reserved for this outcome.
    #[must_use]
    pub const fn fact_id(&self) -> ReplicatedEntityId {
        self.fact_id
    }

    /// Returns the portable media identity retained by the builder.
    #[must_use]
    pub fn media_ref(&self) -> &PortableMediaRef {
        &self.media_ref
    }

    /// Returns the known source duration in milliseconds.
    #[must_use]
    pub const fn duration_ms(&self) -> u64 {
        self.duration_ms
    }

    /// Returns the already-finalized fact, if one exists.
    #[must_use]
    pub fn finalized(&self) -> Option<&FinalizedPlayFact> {
        self.finalized.as_ref().map(|(_, fact)| fact)
    }

    /// Finalizes from the observed rendered position in the shared clock.
    ///
    /// The observed position is converted from microseconds to milliseconds
    /// and bounded by the known item duration. A repeated request with the same
    /// outcome returns the original fact without creating another one. A
    /// different second outcome is rejected as a conflict.
    ///
    /// # Errors
    ///
    /// Returns [`PlaybackFactError::Model`] for invalid model fields, or
    /// [`PlaybackFactError::FinalizationConflict`] for a different duplicate
    /// outcome.
    pub fn finalize_from_clock(
        &mut self,
        clock: PresentationClock,
        outcome: PlaybackOutcome,
    ) -> Result<FinalizeResult, PlaybackFactError> {
        if let Some((existing_outcome, fact)) = &self.finalized {
            if *existing_outcome == outcome {
                return Ok(FinalizeResult::AlreadyFinalized(fact.clone()));
            }
            return Err(PlaybackFactError::FinalizationConflict(
                FinalizationConflict {
                    existing: *existing_outcome,
                    requested: outcome,
                },
            ));
        }

        let played_ms = (clock.media_position_us() / 1_000).min(self.duration_ms);
        let fact = FinalizedPlayFact::new(
            self.fact_id,
            self.media_ref.clone(),
            self.started_at.clone(),
            self.duration_ms,
            played_ms,
            outcome.completion(),
            outcome.end_reason(),
        )?;
        self.finalized = Some((outcome, fact.clone()));
        Ok(FinalizeResult::Created(fact))
    }
}

#[cfg(test)]
mod tests {
    use super::{FinalizeResult, PlaybackFactBuilder, PlaybackFactError, PlaybackOutcome};
    use crate::clock::PresentationClock;
    use aurorix_model::{
        ids::ReplicatedEntityId,
        media_reference::{ExternalEntityType, ExternalIdentity, PortableMediaRef},
    };

    fn media_ref() -> PortableMediaRef {
        let identity = ExternalIdentity::canonical(
            "example.music",
            ExternalEntityType::Recording,
            "recording-1",
        )
        .expect("valid identity");
        PortableMediaRef::provider_recording(identity).expect("valid media reference")
    }

    fn builder(duration_ms: u64) -> PlaybackFactBuilder {
        PlaybackFactBuilder::new(
            ReplicatedEntityId::new_v7(),
            media_ref(),
            "2026-08-28T12:00:00Z",
            duration_ms,
        )
    }

    fn clock_at_frames(frames: u64) -> PresentationClock {
        let mut clock = PresentationClock::new(1_000).expect("valid rate");
        clock
            .advance_rendered_frames(frames)
            .expect("clock can advance");
        clock
    }

    #[test]
    fn rendered_clock_position_generates_one_fact_and_duplicate_is_idempotent() {
        let mut builder = builder(2_000);
        let first = builder
            .finalize_from_clock(clock_at_frames(1_500), PlaybackOutcome::Ended)
            .expect("fact is valid");
        assert!(first.was_created());
        assert_eq!(first.fact().played_ms(), 1_500);
        assert_eq!(first.fact().completion().as_str(), "completed");
        assert_eq!(first.fact().end_reason().as_str(), "ended");

        let second = builder
            .finalize_from_clock(clock_at_frames(1_900), PlaybackOutcome::Ended)
            .expect("same outcome is idempotent");
        assert!(matches!(second, FinalizeResult::AlreadyFinalized(_)));
        assert_eq!(second.fact(), first.fact());
    }

    #[test]
    fn played_position_is_bounded_by_known_duration() {
        let mut builder = builder(1_000);
        let result = builder
            .finalize_from_clock(clock_at_frames(2_000), PlaybackOutcome::Stopped)
            .expect("fact is valid");
        assert_eq!(result.fact().played_ms(), 1_000);
        assert_eq!(result.fact().end_reason().as_str(), "stopped");
    }

    #[test]
    fn all_terminal_outcomes_map_to_the_existing_model_contract() {
        let expected = [
            (PlaybackOutcome::Ended, "completed", "ended"),
            (PlaybackOutcome::Skipped, "partial", "skipped"),
            (PlaybackOutcome::Stopped, "partial", "stopped"),
            (PlaybackOutcome::Error, "partial", "error"),
            (
                PlaybackOutcome::UnknownAfterCrash,
                "unknown_after_crash",
                "crash_recovery",
            ),
        ];
        for (outcome, completion, end_reason) in expected {
            let mut builder = builder(10_000);
            let result = builder
                .finalize_from_clock(clock_at_frames(100), outcome)
                .expect("outcome is supported");
            assert_eq!(result.fact().completion().as_str(), completion);
            assert_eq!(result.fact().end_reason().as_str(), end_reason);
        }
    }

    #[test]
    fn conflicting_second_outcome_is_rejected_without_replacing_the_fact() {
        let mut builder = builder(2_000);
        builder
            .finalize_from_clock(clock_at_frames(500), PlaybackOutcome::Skipped)
            .expect("first finalization");
        let error = builder
            .finalize_from_clock(clock_at_frames(1_000), PlaybackOutcome::Error)
            .expect_err("one outcome owns a builder");
        assert!(matches!(
            error,
            PlaybackFactError::FinalizationConflict(conflict)
                if conflict.existing == PlaybackOutcome::Skipped
                    && conflict.requested == PlaybackOutcome::Error
        ));
        assert_eq!(
            builder
                .finalized()
                .expect("fact retained")
                .end_reason()
                .as_str(),
            "skipped"
        );
    }

    #[test]
    fn invalid_timestamp_is_reported_by_the_existing_model_validator() {
        let mut builder = PlaybackFactBuilder::new(
            ReplicatedEntityId::new_v7(),
            media_ref(),
            "not-a-timestamp",
            100,
        );
        assert!(matches!(
            builder.finalize_from_clock(clock_at_frames(1), PlaybackOutcome::Error),
            Err(PlaybackFactError::Model(_))
        ));
        assert!(builder.finalized().is_none());
    }
}
