//! Immutable, portable playback facts.
//!
//! A finalized fact records one playback outcome for synchronization and
//! statistics. It deliberately contains no local catalog identifier, resource
//! locator, Provider credential, or mutable playback state.

use std::{error::Error, fmt};

use crate::{ids::ReplicatedEntityId, media_reference::PortableMediaRef};

/// The completion classification recorded for one finalized playback.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Completion {
    /// Playback reached its normal end.
    Completed,
    /// Playback stopped before its normal end.
    Partial,
    /// Playback outcome could not be confirmed after a crash.
    UnknownAfterCrash,
}

impl Completion {
    /// Returns the stable wire value used by Sync payloads.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Completed => "completed",
            Self::Partial => "partial",
            Self::UnknownAfterCrash => "unknown_after_crash",
        }
    }
}

/// Alias using the domain-specific name for callers that prefer it.
pub type PlayCompletion = Completion;

/// The terminal reason recorded for one finalized playback.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EndReason {
    /// The track ended normally.
    Ended,
    /// The user or queue skipped the track.
    Skipped,
    /// Playback was explicitly stopped.
    Stopped,
    /// Playback terminated because of an error.
    Error,
    /// The fact was finalized while recovering from a playback crash.
    CrashRecovery,
}

impl EndReason {
    /// Returns the stable wire value used by Sync payloads.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Ended => "ended",
            Self::Skipped => "skipped",
            Self::Stopped => "stopped",
            Self::Error => "error",
            Self::CrashRecovery => "crash_recovery",
        }
    }
}

/// Alias using the domain-specific name for callers that prefer it.
pub type PlayEndReason = EndReason;

/// Validation failure for a finalized playback fact.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlayFactError {
    /// The timestamp was empty or not RFC 3339 date-time syntax.
    InvalidStartedAt,
}

impl fmt::Display for PlayFactError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidStartedAt => {
                formatter.write_str("started_at must be a valid RFC 3339 date-time")
            }
        }
    }
}

impl Error for PlayFactError {}

/// An immutable, append-only playback outcome suitable for replication.
///
/// The fact ID is its de-duplication identity. The media reference is
/// portable; local IDs, file paths, stream locators, and credentials are not
/// accepted by this type because they are not fields of the value at all.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FinalizedPlayFact {
    fact_id: ReplicatedEntityId,
    media_ref: PortableMediaRef,
    started_at: String,
    duration_ms: u64,
    played_ms: u64,
    completion: Completion,
    end_reason: EndReason,
}

impl FinalizedPlayFact {
    /// Creates a validated immutable playback fact.
    ///
    /// # Errors
    ///
    /// Returns [`PlayFactError::InvalidStartedAt`] for a non-RFC 3339
    /// timestamp. Duration and playback values are unsigned by construction;
    /// their semantic relationship is evaluated by the playback/statistics
    /// layer rather than this immutable wire value.
    pub fn new(
        fact_id: ReplicatedEntityId,
        media_ref: PortableMediaRef,
        started_at: impl Into<String>,
        duration_ms: u64,
        played_ms: u64,
        completion: Completion,
        end_reason: EndReason,
    ) -> Result<Self, PlayFactError> {
        let started_at = started_at.into();
        if !is_rfc3339_timestamp(&started_at) {
            return Err(PlayFactError::InvalidStartedAt);
        }
        Ok(Self {
            fact_id,
            media_ref,
            started_at,
            duration_ms,
            played_ms,
            completion,
            end_reason,
        })
    }

    /// Returns the stable fact identity used for de-duplication.
    #[must_use]
    pub const fn fact_id(&self) -> ReplicatedEntityId {
        self.fact_id
    }

    /// Returns the portable media identity associated with the playback.
    #[must_use]
    pub fn media_ref(&self) -> &PortableMediaRef {
        &self.media_ref
    }

    /// Returns the RFC 3339 playback start timestamp.
    #[must_use]
    pub fn started_at(&self) -> &str {
        &self.started_at
    }

    /// Returns the recorded media duration in milliseconds.
    #[must_use]
    pub const fn duration_ms(&self) -> u64 {
        self.duration_ms
    }

    /// Returns the observed playback time in milliseconds.
    #[must_use]
    pub const fn played_ms(&self) -> u64 {
        self.played_ms
    }

    /// Returns the completion classification.
    #[must_use]
    pub const fn completion(&self) -> Completion {
        self.completion
    }

    /// Returns the terminal playback reason.
    #[must_use]
    pub const fn end_reason(&self) -> EndReason {
        self.end_reason
    }
}

fn is_rfc3339_timestamp(value: &str) -> bool {
    let bytes = value.as_bytes();
    if bytes.len() < 20 || value.trim() != value {
        return false;
    }

    // RFC 3339 full-date and partial-time prefix: YYYY-MM-DDTHH:MM:SS.
    if !all_digits(&bytes[0..4])
        || bytes[4] != b'-'
        || !all_digits(&bytes[5..7])
        || bytes[7] != b'-'
        || !all_digits(&bytes[8..10])
        || !(bytes[10] == b'T' || bytes[10] == b't')
        || !all_digits(&bytes[11..13])
        || bytes[13] != b':'
        || !all_digits(&bytes[14..16])
        || bytes[16] != b':'
        || !all_digits(&bytes[17..19])
    {
        return false;
    }

    let month = two_digit_value(&bytes[5..7]);
    let day = two_digit_value(&bytes[8..10]);
    let hour = two_digit_value(&bytes[11..13]);
    let minute = two_digit_value(&bytes[14..16]);
    let second = two_digit_value(&bytes[17..19]);
    if !(1..=12).contains(&month)
        || !(1..=31).contains(&day)
        || hour > 23
        || minute > 59
        || second > 60
    {
        return false;
    }

    let mut index = 19;
    if index < bytes.len() && bytes[index] == b'.' {
        index += 1;
        let fraction_start = index;
        while index < bytes.len() && bytes[index].is_ascii_digit() {
            index += 1;
        }
        if index == fraction_start {
            return false;
        }
    }

    if index == bytes.len() - 1 && (bytes[index] == b'Z' || bytes[index] == b'z') {
        return true;
    }
    if bytes.len().saturating_sub(index) != 6
        || !(bytes[index] == b'+' || bytes[index] == b'-')
        || bytes[index + 3] != b':'
        || !all_digits(&bytes[index + 1..index + 3])
        || !all_digits(&bytes[index + 4..index + 6])
    {
        return false;
    }

    let offset_hour = two_digit_value(&bytes[index + 1..index + 3]);
    let offset_minute = two_digit_value(&bytes[index + 4..index + 6]);
    offset_hour <= 23 && offset_minute <= 59
}

fn all_digits(value: &[u8]) -> bool {
    !value.is_empty() && value.iter().all(u8::is_ascii_digit)
}

fn two_digit_value(value: &[u8]) -> u8 {
    (value[0] - b'0') * 10 + value[1] - b'0'
}

#[cfg(test)]
mod tests {
    use uuid::Uuid;

    use super::{Completion, EndReason, FinalizedPlayFact, PlayFactError};
    use crate::{
        ids::ReplicatedEntityId,
        media_reference::{ExternalEntityType, ExternalIdentity, PortableMediaRef},
    };

    fn fact_id(value: u128) -> ReplicatedEntityId {
        ReplicatedEntityId::from_uuid(Uuid::from_u128(value))
    }

    fn media_ref() -> PortableMediaRef {
        PortableMediaRef::provider_recording(
            ExternalIdentity::canonical(
                "musicbrainz",
                ExternalEntityType::Recording,
                "recording-1",
            )
            .expect("valid external identity"),
        )
        .expect("valid provider reference")
    }

    #[test]
    fn finalized_fact_exposes_immutable_portable_values() {
        let fact = FinalizedPlayFact::new(
            fact_id(1),
            media_ref(),
            "2026-08-27T12:34:56.123Z",
            180_000,
            180_000,
            Completion::Completed,
            EndReason::Ended,
        )
        .expect("valid finalized fact");

        assert_eq!(fact.fact_id(), fact_id(1));
        assert_eq!(fact.started_at(), "2026-08-27T12:34:56.123Z");
        assert_eq!(fact.duration_ms(), 180_000);
        assert_eq!(fact.played_ms(), 180_000);
        assert_eq!(fact.completion(), Completion::Completed);
        assert_eq!(fact.end_reason(), EndReason::Ended);
        assert_eq!(fact.completion().as_str(), "completed");
        assert_eq!(fact.end_reason().as_str(), "ended");
    }

    #[test]
    fn duration_and_played_values_are_non_negative_unsigned_values() {
        let fact = FinalizedPlayFact::new(
            fact_id(2),
            media_ref(),
            "2026-08-27T12:34:56+08:00",
            100,
            u64::MAX,
            Completion::Partial,
            EndReason::Stopped,
        )
        .expect("unsigned duration and playback values are valid");

        assert_eq!(fact.duration_ms(), 100);
        assert_eq!(fact.played_ms(), u64::MAX);
    }

    #[test]
    fn invalid_timestamp_is_rejected() {
        assert_eq!(
            FinalizedPlayFact::new(
                fact_id(3),
                media_ref(),
                "2026-08-27 12:34:56Z",
                100,
                50,
                Completion::Partial,
                EndReason::Stopped,
            ),
            Err(PlayFactError::InvalidStartedAt)
        );
    }

    #[test]
    fn all_completion_and_end_reason_values_have_stable_wire_values() {
        let fact = FinalizedPlayFact::new(
            fact_id(5),
            media_ref(),
            "2026-08-27T12:34:56Z",
            100,
            50,
            Completion::UnknownAfterCrash,
            EndReason::Error,
        )
        .expect("completion and end reason are independently classified");
        assert_eq!(fact.completion(), Completion::UnknownAfterCrash);
        assert_eq!(fact.end_reason(), EndReason::Error);

        assert_eq!(Completion::Partial.as_str(), "partial");
        assert_eq!(
            Completion::UnknownAfterCrash.as_str(),
            "unknown_after_crash"
        );
        assert_eq!(EndReason::Skipped.as_str(), "skipped");
        assert_eq!(EndReason::Stopped.as_str(), "stopped");
        assert_eq!(EndReason::Error.as_str(), "error");
        assert_eq!(EndReason::CrashRecovery.as_str(), "crash_recovery");
    }
}
