//! Playback commands, request identities, and worker-result classification.
//!
//! This module is deliberately independent of a platform audio backend.  A
//! request ID identifies one caller command for the lifetime of a playback
//! session; it is not a durable Sync operation ID and it is not persisted as
//! part of media state.

use std::{collections::BTreeMap, error::Error, fmt};

/// An opaque caller-supplied identifier for one playback command.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RequestId(u64);

impl RequestId {
    /// Creates a request ID from a caller-owned value.
    #[must_use]
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    /// Returns the value for adapters that must serialize the ID.
    #[must_use]
    pub const fn value(self) -> u64 {
        self.0
    }
}

impl From<u64> for RequestId {
    fn from(value: u64) -> Self {
        Self::new(value)
    }
}

impl fmt::Display for RequestId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

/// An opaque Core media identity held by the playback queue.
///
/// A playback item contains no path, descriptor, URL, credential, or runtime
/// lease.  Those values are resolved by a later source boundary.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PlaybackItemId(String);

impl PlaybackItemId {
    /// Creates a non-empty normalized playback item identity.
    ///
    /// # Errors
    ///
    /// Returns [`PlaybackItemIdError::Empty`] for a blank identity.
    pub fn new(value: impl Into<String>) -> Result<Self, PlaybackItemIdError> {
        let value = value.into();
        let normalized = value.trim();
        if normalized.is_empty() {
            return Err(PlaybackItemIdError::Empty);
        }

        Ok(Self(normalized.to_owned()))
    }

    /// Returns the normalized opaque identity.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<&str> for PlaybackItemId {
    type Error = PlaybackItemIdError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl TryFrom<String> for PlaybackItemId {
    type Error = PlaybackItemIdError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl fmt::Display for PlaybackItemId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

/// Validation failure for a playback item identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlaybackItemIdError {
    /// The identity contained no non-whitespace characters.
    Empty,
}

impl fmt::Display for PlaybackItemIdError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => formatter.write_str("playback item ID must not be empty"),
        }
    }
}

impl Error for PlaybackItemIdError {}

/// The queue repeat policy carried by the command boundary.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum RepeatMode {
    /// Stop at the end of the queue.
    #[default]
    Off,
    /// Replay the current item after normal completion.
    One,
    /// Wrap to the deterministic first queue item.
    All,
}

impl RepeatMode {
    /// Returns the stable lower-case contract value.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::One => "one",
            Self::All => "all",
        }
    }
}

/// One action issued through the Core playback command boundary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlaybackAction {
    /// Resolve and start the supplied item, or the current item when omitted.
    Play {
        /// An optional item selected by the caller.
        item_id: Option<PlaybackItemId>,
    },
    /// Stop consuming rendered frames while retaining the media position.
    Pause,
    /// Resume after the output path is ready.
    Resume,
    /// Perform a worker-side seek in media microseconds.
    Seek {
        /// The requested non-negative media position.
        position_us: u64,
    },
    /// End the active session and reset the presentation position.
    Stop,
    /// Select the next item according to the queue policy.
    Next,
    /// Select the previous item according to the queue policy.
    Previous,
    /// Replace queue intent.  Runtime resources are not part of this value.
    SetQueue {
        /// Ordered Core media identities.
        items: Vec<PlaybackItemId>,
        /// Optional current item index in the replacement queue.
        current_index: Option<usize>,
    },
    /// Enable or disable deterministic shuffle using the supplied seed.
    SetShuffle {
        /// Whether shuffle is enabled.
        enabled: bool,
        /// The explicit deterministic permutation seed.
        seed: u64,
    },
    /// Change the queue repeat policy.
    SetRepeat {
        /// The new repeat policy.
        mode: RepeatMode,
    },
    /// Cancel a not-yet-effective worker operation.
    Cancel {
        /// The request ID of the operation to cancel.
        target_request_id: RequestId,
    },
}

/// A caller command with its correlation identity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlaybackCommand {
    request_id: RequestId,
    action: PlaybackAction,
}

impl PlaybackCommand {
    /// Creates a command with an explicit caller request ID.
    #[must_use]
    pub const fn new(request_id: RequestId, action: PlaybackAction) -> Self {
        Self { request_id, action }
    }

    /// Returns the caller request ID.
    #[must_use]
    pub const fn request_id(&self) -> RequestId {
        self.request_id
    }

    /// Returns the command action without transferring ownership.
    #[must_use]
    pub const fn action(&self) -> &PlaybackAction {
        &self.action
    }

    /// Splits the command into its identity and action.
    #[must_use]
    pub fn into_parts(self) -> (RequestId, PlaybackAction) {
        (self.request_id, self.action)
    }
}

/// A token attached to an asynchronous worker operation.
///
/// The revision and buffer generation prevent a late worker result from
/// writing into a newer session timeline.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct OperationToken {
    request_id: RequestId,
    revision: u64,
    buffer_generation: u64,
}

impl OperationToken {
    /// Creates a worker token.  Session code normally obtains tokens from
    /// [`RequestTracker::start`].
    #[must_use]
    pub const fn new(request_id: RequestId, revision: u64, buffer_generation: u64) -> Self {
        Self {
            request_id,
            revision,
            buffer_generation,
        }
    }

    /// Returns the request ID represented by this token.
    #[must_use]
    pub const fn request_id(self) -> RequestId {
        self.request_id
    }

    /// Returns the monotonically increasing accepted-command revision.
    #[must_use]
    pub const fn revision(self) -> u64 {
        self.revision
    }

    /// Returns the buffer generation captured when the operation was issued.
    #[must_use]
    pub const fn buffer_generation(self) -> u64 {
        self.buffer_generation
    }

    /// Returns an equivalent token for a newly activated buffer generation.
    #[must_use]
    pub const fn with_buffer_generation(self, buffer_generation: u64) -> Self {
        Self {
            request_id: self.request_id,
            revision: self.revision,
            buffer_generation,
        }
    }
}

/// The reason a worker result is no longer eligible to mutate the session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StaleReason {
    /// No request with the token's ID exists in this session.
    UnknownRequest,
    /// A newer accepted command superseded this operation.
    RequestSuperseded,
    /// The session retired the buffer generation captured by this operation.
    BufferGenerationChanged,
    /// The request was already finalized or rejected.
    RequestNoLongerPending,
    /// The worker supplied a payload inconsistent with the pending request.
    PayloadMismatch,
}

/// The result of asking the request ledger to cancel an operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CancellationOutcome {
    /// The worker operation was pending and is now cancelled before commit.
    CancelledBeforeCommit,
    /// The operation had already been applied; cancellation cannot roll it back.
    AlreadyApplied,
    /// The same target was already cancelled.
    AlreadyCancelled,
    /// The target was superseded, so its final worker outcome is unknown.
    OutcomeUnknown,
    /// No target with this request ID was recorded.
    UnknownRequest,
}

/// Why the Core rejected a command at the control boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RejectionReason {
    /// The command does not apply in the current session state.
    InvalidState,
    /// An action requiring a current item had none.
    NoCurrentItem,
    /// This request ID was already used.
    DuplicateRequest,
    /// The cancel target was not recorded.
    UnknownCancellationTarget,
    /// The target was already cancelled and cannot be cancelled again.
    AlreadyCancelled,
    /// Queue policy is owned by a later Gate 2 batch.
    QueuePolicyNotReady,
    /// The queue payload is structurally invalid.
    InvalidQueue,
    /// A worker event did not match the pending operation phase.
    UnexpectedWorkerEvent,
}

/// A classified result shared by command replies and worker-event handling.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ResultDisposition {
    /// The command or worker result is current and accepted.
    Accepted {
        /// The accepted command revision.
        revision: u64,
    },
    /// Cancellation was handled without pretending to roll back state.
    Cancelled {
        /// The operation targeted by cancellation.
        target_request_id: RequestId,
        /// The precise cancellation outcome.
        outcome: CancellationOutcome,
    },
    /// The command or worker event was rejected.
    Rejected {
        /// The stable rejection reason.
        reason: RejectionReason,
    },
    /// The result must be ignored by the session coordinator.
    Stale {
        /// The reason the result is no longer current.
        reason: StaleReason,
    },
}

/// The classified result returned for a caller command.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CommandResult {
    request_id: RequestId,
    disposition: ResultDisposition,
}

impl CommandResult {
    /// Creates a classified command result.
    #[must_use]
    pub const fn new(request_id: RequestId, disposition: ResultDisposition) -> Self {
        Self {
            request_id,
            disposition,
        }
    }

    /// Returns the command request ID.
    #[must_use]
    pub const fn request_id(self) -> RequestId {
        self.request_id
    }

    /// Returns the result classification.
    #[must_use]
    pub const fn disposition(self) -> ResultDisposition {
        self.disposition
    }

    /// Reports whether the command was accepted for execution.
    #[must_use]
    pub const fn is_accepted(self) -> bool {
        matches!(self.disposition, ResultDisposition::Accepted { .. })
    }
}

/// Alias used by callers that refer to command results as outcomes.
pub type CommandOutcome = ResultDisposition;

/// Failure while reserving a request revision.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RequestStartError {
    /// The request ID already exists in this session.
    DuplicateRequest,
    /// The monotonic revision reached its representable limit.
    RevisionExhausted,
}

impl fmt::Display for RequestStartError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DuplicateRequest => formatter.write_str("request ID is already recorded"),
            Self::RevisionExhausted => formatter.write_str("request revision is exhausted"),
        }
    }
}

impl Error for RequestStartError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RequestState {
    Active,
    Applied,
    Rejected,
    Cancelled(CancellationOutcome),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct RequestRecord {
    revision: u64,
    state: RequestState,
}

/// Session-local request ledger used to classify cancellation and stale data.
#[derive(Debug, Default)]
pub struct RequestTracker {
    next_revision: u64,
    latest_revision: u64,
    records: BTreeMap<RequestId, RequestRecord>,
}

impl RequestTracker {
    /// Creates an empty request ledger.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns whether a request ID has already been consumed.
    #[must_use]
    pub fn contains(&self, request_id: RequestId) -> bool {
        self.records.contains_key(&request_id)
    }

    /// Reserves an accepted command revision and returns its worker token.
    ///
    /// # Errors
    ///
    /// Returns [`RequestStartError::DuplicateRequest`] for a reused ID or
    /// [`RequestStartError::RevisionExhausted`] at the integer limit.
    pub fn start(
        &mut self,
        request_id: RequestId,
        buffer_generation: u64,
    ) -> Result<OperationToken, RequestStartError> {
        if self.contains(request_id) {
            return Err(RequestStartError::DuplicateRequest);
        }

        let revision = self
            .next_revision
            .checked_add(1)
            .ok_or(RequestStartError::RevisionExhausted)?;
        self.next_revision = revision;
        self.latest_revision = revision;
        self.records.insert(
            request_id,
            RequestRecord {
                revision,
                state: RequestState::Active,
            },
        );
        Ok(OperationToken::new(request_id, revision, buffer_generation))
    }

    /// Records a rejected command without superseding an active operation.
    ///
    /// # Errors
    ///
    /// Returns [`RequestStartError::DuplicateRequest`] or
    /// [`RequestStartError::RevisionExhausted`] when the ID cannot be recorded.
    pub fn record_rejected(&mut self, request_id: RequestId) -> Result<(), RequestStartError> {
        if self.contains(request_id) {
            return Err(RequestStartError::DuplicateRequest);
        }

        let revision = self
            .next_revision
            .checked_add(1)
            .ok_or(RequestStartError::RevisionExhausted)?;
        self.next_revision = revision;
        self.records.insert(
            request_id,
            RequestRecord {
                revision,
                state: RequestState::Rejected,
            },
        );
        Ok(())
    }

    /// Records a synchronously applied command without superseding a pending
    /// worker operation.
    ///
    /// # Errors
    ///
    /// Returns [`RequestStartError::DuplicateRequest`] or
    /// [`RequestStartError::RevisionExhausted`] when the ID cannot be recorded.
    pub fn record_applied(
        &mut self,
        request_id: RequestId,
        buffer_generation: u64,
    ) -> Result<OperationToken, RequestStartError> {
        if self.contains(request_id) {
            return Err(RequestStartError::DuplicateRequest);
        }

        let revision = self
            .next_revision
            .checked_add(1)
            .ok_or(RequestStartError::RevisionExhausted)?;
        self.next_revision = revision;
        self.records.insert(
            request_id,
            RequestRecord {
                revision,
                state: RequestState::Applied,
            },
        );
        Ok(OperationToken::new(request_id, revision, buffer_generation))
    }

    /// Marks an accepted operation as having taken effect.
    #[must_use]
    pub fn mark_applied(&mut self, request_id: RequestId) -> bool {
        let Some(record) = self.records.get_mut(&request_id) else {
            return false;
        };
        if record.state != RequestState::Active {
            return false;
        }
        record.state = RequestState::Applied;
        true
    }

    /// Returns the precise outcome that a cancellation request would observe.
    #[must_use]
    pub fn cancellation_outcome(&self, request_id: RequestId) -> CancellationOutcome {
        let Some(record) = self.records.get(&request_id) else {
            return CancellationOutcome::UnknownRequest;
        };
        match record.state {
            RequestState::Active if record.revision == self.latest_revision => {
                CancellationOutcome::CancelledBeforeCommit
            }
            RequestState::Active => CancellationOutcome::OutcomeUnknown,
            RequestState::Applied | RequestState::Rejected => CancellationOutcome::AlreadyApplied,
            RequestState::Cancelled(_) => CancellationOutcome::AlreadyCancelled,
        }
    }

    /// Cancels a target without rolling back any already-applied state.
    #[must_use]
    pub fn cancel(&mut self, request_id: RequestId) -> CancellationOutcome {
        let Some(record) = self.records.get_mut(&request_id) else {
            return CancellationOutcome::UnknownRequest;
        };

        let outcome = match record.state {
            RequestState::Active if record.revision == self.latest_revision => {
                CancellationOutcome::CancelledBeforeCommit
            }
            RequestState::Active => CancellationOutcome::OutcomeUnknown,
            RequestState::Applied | RequestState::Rejected => CancellationOutcome::AlreadyApplied,
            RequestState::Cancelled(_) => CancellationOutcome::AlreadyCancelled,
        };
        if matches!(
            outcome,
            CancellationOutcome::CancelledBeforeCommit | CancellationOutcome::OutcomeUnknown
        ) {
            record.state = RequestState::Cancelled(outcome);
        }
        outcome
    }

    /// Classifies a worker token against the current request and buffer state.
    #[must_use]
    pub fn classify(
        &self,
        token: OperationToken,
        current_buffer_generation: u64,
    ) -> ResultDisposition {
        let Some(record) = self.records.get(&token.request_id) else {
            return ResultDisposition::Stale {
                reason: StaleReason::UnknownRequest,
            };
        };

        if let RequestState::Cancelled(outcome) = record.state {
            return ResultDisposition::Cancelled {
                target_request_id: token.request_id,
                outcome,
            };
        }

        if token.buffer_generation != current_buffer_generation {
            return ResultDisposition::Stale {
                reason: StaleReason::BufferGenerationChanged,
            };
        }
        if record.revision != self.latest_revision {
            return ResultDisposition::Stale {
                reason: StaleReason::RequestSuperseded,
            };
        }

        match record.state {
            RequestState::Active => ResultDisposition::Accepted {
                revision: record.revision,
            },
            RequestState::Applied | RequestState::Rejected => ResultDisposition::Stale {
                reason: StaleReason::RequestNoLongerPending,
            },
            RequestState::Cancelled(_) => unreachable!("cancelled state handled above"),
        }
    }

    /// Returns the latest accepted command revision.
    #[must_use]
    pub const fn latest_revision(&self) -> u64 {
        self.latest_revision
    }
}
