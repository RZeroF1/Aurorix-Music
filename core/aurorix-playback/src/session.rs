//! Deterministic playback-session state machine.
//!
//! The session owns playback intent and the presentation clock.  Source
//! opening, decoding, buffering, and realtime output are represented here by
//! worker intents/events only; they are implemented by later audio modules.

use std::{error::Error, fmt};

use crate::{
    clock::{ClockError, PresentationClock},
    command::{
        CancellationOutcome, CommandResult, OperationToken, PlaybackAction, PlaybackCommand,
        PlaybackItemId, RejectionReason, RequestId, RequestStartError, RequestTracker,
        ResultDisposition, StaleReason,
    },
};

/// The observable state of one local playback session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SessionState {
    /// No current item has been selected.
    Empty,
    /// A source is being resolved or reopened by a worker.
    Loading,
    /// A source exists but the output path is waiting for enough data.
    Buffering,
    /// The output boundary is consuming rendered frames.
    Playing,
    /// Output consumption is stopped while the position is retained.
    Paused,
    /// The session was explicitly stopped.
    Stopped,
    /// The current item reached its terminal end.
    Ended,
    /// The current item failed during source or decoder execution.
    Failed,
    /// The current item has no currently available runtime source.
    Unavailable,
}

impl SessionState {
    /// Reports whether the state has a current active or pending operation.
    #[must_use]
    pub const fn is_active(self) -> bool {
        matches!(
            self,
            Self::Loading | Self::Buffering | Self::Playing | Self::Paused
        )
    }

    /// Reports whether the state is a terminal outcome for the current item.
    #[must_use]
    pub const fn is_terminal(self) -> bool {
        matches!(self, Self::Ended | Self::Failed | Self::Unavailable)
    }
}

/// A worker operation requested by the session coordinator.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkerIntent {
    /// Resolve and open the current item outside the realtime path.
    ResolveSource {
        /// Request and generation fence.
        token: OperationToken,
        /// Core identity to resolve.
        item_id: PlaybackItemId,
    },
    /// Fill enough data for the output boundary to become active.
    PrepareBuffer {
        /// Request and generation fence.
        token: OperationToken,
        /// Core identity being prepared.
        item_id: PlaybackItemId,
        /// Position from which preparation begins.
        position_us: u64,
    },
    /// Perform a worker-side seek and retire the previous buffer generation.
    Seek {
        /// Request and generation fence.
        token: OperationToken,
        /// Target media position.
        position_us: u64,
    },
    /// Stop consumption and close the active runtime operation.
    Pause {
        /// Request and generation fence.
        token: OperationToken,
    },
    /// Stop and retire the active runtime operation.
    Stop {
        /// Request and generation fence.
        token: OperationToken,
    },
}

impl WorkerIntent {
    /// Returns the fence token carried by this intent.
    #[must_use]
    pub const fn token(&self) -> OperationToken {
        match self {
            Self::ResolveSource { token, .. }
            | Self::PrepareBuffer { token, .. }
            | Self::Seek { token, .. }
            | Self::Pause { token }
            | Self::Stop { token } => *token,
        }
    }
}

/// A result/event emitted by a non-realtime worker.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkerEvent {
    /// The source was opened and the worker can begin prebuffering.
    SourceReady {
        /// Fence token from the originating intent.
        token: OperationToken,
    },
    /// The output path has enough data to consume frames.
    PrebufferReady {
        /// Fence token from the originating intent.
        token: OperationToken,
    },
    /// A seek took effect at the worker and supplied the effective position.
    SeekApplied {
        /// Fence token from the originating intent.
        token: OperationToken,
        /// Effective media position.
        position_us: u64,
    },
    /// The runtime source is unavailable while the Core identity remains valid.
    SourceUnavailable {
        /// Fence token from the originating intent.
        token: OperationToken,
    },
    /// The worker failed the current operation.
    Failed {
        /// Fence token from the originating intent.
        token: OperationToken,
    },
    /// The current item reached its normal end.
    Ended {
        /// Fence token from the originating intent.
        token: OperationToken,
    },
}

impl WorkerEvent {
    /// Returns the fence token carried by this event.
    #[must_use]
    pub const fn token(&self) -> OperationToken {
        match self {
            Self::SourceReady { token }
            | Self::PrebufferReady { token }
            | Self::SeekApplied { token, .. }
            | Self::SourceUnavailable { token }
            | Self::Failed { token }
            | Self::Ended { token } => *token,
        }
    }

    /// Returns the terminal session state represented by this event.
    #[must_use]
    pub const fn terminal_state(&self) -> Option<SessionState> {
        match self {
            Self::SourceUnavailable { .. } => Some(SessionState::Unavailable),
            Self::Failed { .. } => Some(SessionState::Failed),
            Self::Ended { .. } => Some(SessionState::Ended),
            Self::SourceReady { .. } | Self::PrebufferReady { .. } | Self::SeekApplied { .. } => {
                None
            }
        }
    }
}

/// A bounded latest-value view consumed by clients and later platform hosts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlaybackSnapshot {
    state: SessionState,
    current_item: Option<PlaybackItemId>,
    clock: PresentationClock,
    state_version: u64,
    buffer_generation: u64,
    pending_request_id: Option<RequestId>,
}

impl PlaybackSnapshot {
    /// Returns the session state.
    #[must_use]
    pub const fn state(&self) -> SessionState {
        self.state
    }

    /// Returns the current Core media identity, if selected.
    #[must_use]
    pub fn current_item(&self) -> Option<&PlaybackItemId> {
        self.current_item.as_ref()
    }

    /// Returns the latest presentation-clock value.
    #[must_use]
    pub const fn clock(&self) -> PresentationClock {
        self.clock
    }

    /// Returns the monotonically increasing state revision.
    #[must_use]
    pub const fn state_version(&self) -> u64 {
        self.state_version
    }

    /// Returns the active buffer generation.
    #[must_use]
    pub const fn buffer_generation(&self) -> u64 {
        self.buffer_generation
    }

    /// Returns the pending worker request, if any.
    #[must_use]
    pub const fn pending_request_id(&self) -> Option<RequestId> {
        self.pending_request_id
    }
}

/// The outcome of applying one command, including any worker work to schedule.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionUpdate {
    result: CommandResult,
    intent: Option<WorkerIntent>,
    snapshot: PlaybackSnapshot,
}

impl SessionUpdate {
    /// Returns the classified command result.
    #[must_use]
    pub const fn result(&self) -> CommandResult {
        self.result
    }

    /// Returns the worker intent, if the command requires asynchronous work.
    #[must_use]
    pub fn intent(&self) -> Option<&WorkerIntent> {
        self.intent.as_ref()
    }

    /// Returns the post-command snapshot.
    #[must_use]
    pub const fn snapshot(&self) -> &PlaybackSnapshot {
        &self.snapshot
    }
}

/// The outcome of applying one worker event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkerUpdate {
    disposition: ResultDisposition,
    intent: Option<WorkerIntent>,
    snapshot: PlaybackSnapshot,
}

impl WorkerUpdate {
    /// Returns the event classification.
    #[must_use]
    pub const fn disposition(&self) -> ResultDisposition {
        self.disposition
    }

    /// Returns a follow-up worker intent, if another phase is required.
    #[must_use]
    pub fn intent(&self) -> Option<&WorkerIntent> {
        self.intent.as_ref()
    }

    /// Returns the post-event snapshot.
    #[must_use]
    pub const fn snapshot(&self) -> &PlaybackSnapshot {
        &self.snapshot
    }
}

/// Internal errors that prevent a state update from being represented safely.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionError {
    /// A presentation-clock operation failed its checked arithmetic.
    Clock(ClockError),
    /// The session request ledger could not record a command.
    RequestTracker(RequestStartError),
    /// The state revision cannot be incremented.
    StateVersionExhausted,
    /// The buffer generation cannot be incremented.
    BufferGenerationExhausted,
}

impl fmt::Display for SessionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Clock(error) => write!(formatter, "presentation clock error: {error}"),
            Self::RequestTracker(error) => write!(formatter, "request tracker error: {error}"),
            Self::StateVersionExhausted => {
                formatter.write_str("session state version is exhausted")
            }
            Self::BufferGenerationExhausted => {
                formatter.write_str("playback buffer generation is exhausted")
            }
        }
    }
}

impl Error for SessionError {}

impl From<ClockError> for SessionError {
    fn from(error: ClockError) -> Self {
        Self::Clock(error)
    }
}

impl From<RequestStartError> for SessionError {
    fn from(error: RequestStartError) -> Self {
        Self::RequestTracker(error)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PendingOperation {
    Play {
        token: OperationToken,
        resumes_from_pause: bool,
    },
    Resume {
        token: OperationToken,
    },
    Seek {
        token: OperationToken,
        position_us: u64,
        prior_state: SessionState,
        applied: bool,
    },
}

impl PendingOperation {
    const fn token(self) -> OperationToken {
        match self {
            Self::Play { token, .. } | Self::Resume { token } | Self::Seek { token, .. } => token,
        }
    }
}

/// Core-owned state and presentation-clock coordinator for one session.
#[derive(Debug)]
pub struct PlaybackSession {
    state: SessionState,
    current_item: Option<PlaybackItemId>,
    clock: PresentationClock,
    state_version: u64,
    buffer_generation: u64,
    pending: Option<PendingOperation>,
    active_token: Option<OperationToken>,
    requests: RequestTracker,
}

impl PlaybackSession {
    /// Creates an empty session with a platform-neutral output sample rate.
    ///
    /// # Errors
    ///
    /// Returns [`SessionError::Clock`] when the sample rate is invalid.
    pub fn new(output_sample_rate: u32) -> Result<Self, SessionError> {
        Ok(Self::with_clock(PresentationClock::new(
            output_sample_rate,
        )?))
    }

    /// Creates an empty session around an already validated clock.
    #[must_use]
    pub fn with_clock(clock: PresentationClock) -> Self {
        Self {
            state: SessionState::Empty,
            current_item: None,
            clock,
            state_version: 0,
            buffer_generation: 0,
            pending: None,
            active_token: None,
            requests: RequestTracker::new(),
        }
    }

    /// Returns the current bounded session snapshot.
    #[must_use]
    pub fn snapshot(&self) -> PlaybackSnapshot {
        self.make_snapshot()
    }

    /// Returns the current session state.
    #[must_use]
    pub const fn state(&self) -> SessionState {
        self.state
    }

    /// Returns the current item identity, if one is selected.
    #[must_use]
    pub fn current_item(&self) -> Option<&PlaybackItemId> {
        self.current_item.as_ref()
    }

    /// Returns a copy of the Core-owned presentation clock.
    #[must_use]
    pub const fn clock(&self) -> PresentationClock {
        self.clock
    }

    /// Dispatches one caller command through the deterministic control reducer.
    ///
    /// A successful return means the command was classified and the snapshot
    /// was produced.  Worker intents are advisory data-plane work; they never
    /// execute on the realtime callback.
    ///
    /// # Errors
    ///
    /// Returns [`SessionError`] only when checked internal counters or clock
    /// arithmetic cannot represent the requested transition.
    pub fn dispatch(&mut self, command: PlaybackCommand) -> Result<SessionUpdate, SessionError> {
        let (request_id, action) = command.into_parts();
        if self.requests.contains(request_id) {
            return Ok(self.session_update(
                CommandResult::new(
                    request_id,
                    ResultDisposition::Rejected {
                        reason: RejectionReason::DuplicateRequest,
                    },
                ),
                None,
            ));
        }

        match action {
            PlaybackAction::Play { item_id } => self.dispatch_play(request_id, item_id),
            PlaybackAction::Pause => self.dispatch_pause(request_id),
            PlaybackAction::Resume => self.dispatch_resume(request_id),
            PlaybackAction::Seek { position_us } => self.dispatch_seek(request_id, position_us),
            PlaybackAction::Stop => self.dispatch_stop(request_id),
            PlaybackAction::Cancel { target_request_id } => {
                self.dispatch_cancel(request_id, target_request_id)
            }
            PlaybackAction::Next
            | PlaybackAction::Previous
            | PlaybackAction::SetQueue { .. }
            | PlaybackAction::SetShuffle { .. }
            | PlaybackAction::SetRepeat { .. } => {
                self.rejected_update(request_id, RejectionReason::QueuePolicyNotReady)
            }
        }
    }

    /// Applies a non-realtime worker event and rejects stale results without
    /// mutating the current session.
    ///
    /// # Errors
    ///
    /// Returns [`SessionError`] if applying the event requires an unrepresentable
    /// state or clock transition.
    pub fn handle_worker_event(
        &mut self,
        event: WorkerEvent,
    ) -> Result<WorkerUpdate, SessionError> {
        let token = event.token();
        if let Some(state) = event.terminal_state()
            && self.active_token == Some(token)
        {
            if token.buffer_generation() != self.buffer_generation {
                return Ok(self.worker_update(
                    ResultDisposition::Stale {
                        reason: StaleReason::BufferGenerationChanged,
                    },
                    None,
                ));
            }
            return self.apply_active_terminal_event(token, state);
        }
        let disposition = self.requests.classify(token, self.buffer_generation);
        if !matches!(disposition, ResultDisposition::Accepted { .. }) {
            return Ok(self.worker_update(disposition, None));
        }

        let Some(pending) = self.pending else {
            return Ok(self.worker_update(
                ResultDisposition::Stale {
                    reason: StaleReason::RequestNoLongerPending,
                },
                None,
            ));
        };
        if pending.token() != token {
            return Ok(self.worker_update(
                ResultDisposition::Stale {
                    reason: StaleReason::RequestSuperseded,
                },
                None,
            ));
        }

        match event {
            WorkerEvent::SourceReady { .. } => self.apply_source_ready(pending),
            WorkerEvent::PrebufferReady { .. } => self.apply_prebuffer_ready(pending),
            WorkerEvent::SeekApplied { position_us, .. } => {
                self.apply_seek_applied(pending, position_us)
            }
            WorkerEvent::SourceUnavailable { .. } => {
                self.apply_terminal_worker_event(pending, SessionState::Unavailable)
            }
            WorkerEvent::Failed { .. } => {
                self.apply_terminal_worker_event(pending, SessionState::Failed)
            }
            WorkerEvent::Ended { .. } => {
                self.apply_terminal_worker_event(pending, SessionState::Ended)
            }
        }
    }

    /// Accounts for frames actually consumed by the output boundary.
    ///
    /// Calls made while paused or outside `Playing` are ignored, which keeps
    /// the retained presentation position stable.
    ///
    /// # Errors
    ///
    /// Returns a checked clock error if rendered-frame arithmetic overflows.
    pub fn record_rendered_frames(
        &mut self,
        frames: u64,
    ) -> Result<PlaybackSnapshot, SessionError> {
        if self.state == SessionState::Playing {
            self.clock.advance_rendered_frames(frames)?;
        }
        Ok(self.make_snapshot())
    }

    /// Acknowledges the latest clock discontinuity for snapshot consumers.
    pub fn acknowledge_discontinuity(&mut self) {
        self.clock.acknowledge_discontinuity();
    }

    fn dispatch_play(
        &mut self,
        request_id: RequestId,
        item_id: Option<PlaybackItemId>,
    ) -> Result<SessionUpdate, SessionError> {
        if !matches!(
            self.state,
            SessionState::Empty
                | SessionState::Paused
                | SessionState::Stopped
                | SessionState::Ended
                | SessionState::Failed
                | SessionState::Unavailable
        ) {
            return self.rejected_update(request_id, RejectionReason::InvalidState);
        }

        let selected_item = item_id.or_else(|| self.current_item.clone());
        let Some(selected_item) = selected_item else {
            return self.rejected_update(request_id, RejectionReason::NoCurrentItem);
        };
        let was_paused = self.state == SessionState::Paused;
        let item_changed = self.current_item.as_ref() != Some(&selected_item);
        let token = self.start_request(request_id)?;
        self.current_item = Some(selected_item.clone());
        self.bump_buffer_generation()?;
        let token = token.with_buffer_generation(self.buffer_generation);
        self.pending = Some(PendingOperation::Play {
            token,
            resumes_from_pause: was_paused && !item_changed,
        });
        self.active_token = None;
        self.state = SessionState::Loading;
        self.bump_state_version()?;
        Ok(self.session_update(
            CommandResult::new(
                request_id,
                ResultDisposition::Accepted {
                    revision: token.revision(),
                },
            ),
            Some(WorkerIntent::ResolveSource {
                token,
                item_id: selected_item,
            }),
        ))
    }

    fn dispatch_pause(&mut self, request_id: RequestId) -> Result<SessionUpdate, SessionError> {
        if matches!(self.state, SessionState::Empty) {
            return self.rejected_update(request_id, RejectionReason::InvalidState);
        }
        let mut token = self.record_applied_request(request_id)?;
        let requires_boundary = matches!(
            self.state,
            SessionState::Loading | SessionState::Buffering | SessionState::Playing
        );
        if requires_boundary {
            self.clock.pause_boundary()?;
            self.bump_buffer_generation()?;
            token = token.with_buffer_generation(self.buffer_generation);
            self.pending = None;
            self.active_token = None;
            self.state = SessionState::Paused;
            self.bump_state_version()?;
        }
        Ok(self.session_update(
            CommandResult::new(
                request_id,
                ResultDisposition::Accepted {
                    revision: token.revision(),
                },
            ),
            Some(WorkerIntent::Pause { token }),
        ))
    }

    fn dispatch_resume(&mut self, request_id: RequestId) -> Result<SessionUpdate, SessionError> {
        if self.state == SessionState::Playing {
            let token = self.record_applied_request(request_id)?;
            return Ok(self.session_update(
                CommandResult::new(
                    request_id,
                    ResultDisposition::Accepted {
                        revision: token.revision(),
                    },
                ),
                None,
            ));
        }
        if self.state != SessionState::Paused {
            return self.rejected_update(request_id, RejectionReason::InvalidState);
        }
        let Some(item_id) = self.current_item.clone() else {
            return self.rejected_update(request_id, RejectionReason::NoCurrentItem);
        };
        let token = self.start_request(request_id)?;
        self.bump_buffer_generation()?;
        let token = token.with_buffer_generation(self.buffer_generation);
        self.pending = Some(PendingOperation::Resume { token });
        self.active_token = None;
        self.state = SessionState::Buffering;
        self.bump_state_version()?;
        Ok(self.session_update(
            CommandResult::new(
                request_id,
                ResultDisposition::Accepted {
                    revision: token.revision(),
                },
            ),
            Some(WorkerIntent::PrepareBuffer {
                token,
                item_id,
                position_us: self.clock.media_position_us(),
            }),
        ))
    }

    fn dispatch_seek(
        &mut self,
        request_id: RequestId,
        position_us: u64,
    ) -> Result<SessionUpdate, SessionError> {
        if !matches!(
            self.state,
            SessionState::Loading
                | SessionState::Buffering
                | SessionState::Playing
                | SessionState::Paused
        ) {
            return self.rejected_update(request_id, RejectionReason::InvalidState);
        }
        if self.current_item.is_none() {
            return self.rejected_update(request_id, RejectionReason::NoCurrentItem);
        }
        let prior_state = self.state;
        let token = self.start_request(request_id)?;
        self.bump_buffer_generation()?;
        let token = token.with_buffer_generation(self.buffer_generation);
        self.pending = Some(PendingOperation::Seek {
            token,
            position_us,
            prior_state,
            applied: false,
        });
        self.active_token = None;
        if prior_state != SessionState::Paused {
            self.state = SessionState::Buffering;
        }
        self.bump_state_version()?;
        Ok(self.session_update(
            CommandResult::new(
                request_id,
                ResultDisposition::Accepted {
                    revision: token.revision(),
                },
            ),
            Some(WorkerIntent::Seek { token, position_us }),
        ))
    }

    fn dispatch_stop(&mut self, request_id: RequestId) -> Result<SessionUpdate, SessionError> {
        let mut token = self.record_applied_request(request_id)?;
        if !matches!(self.state, SessionState::Empty | SessionState::Stopped) {
            self.bump_buffer_generation()?;
            token = token.with_buffer_generation(self.buffer_generation);
            self.pending = None;
            self.active_token = None;
            self.clock.stop_boundary()?;
            self.state = SessionState::Stopped;
            self.bump_state_version()?;
        }
        Ok(self.session_update(
            CommandResult::new(
                request_id,
                ResultDisposition::Accepted {
                    revision: token.revision(),
                },
            ),
            Some(WorkerIntent::Stop { token }),
        ))
    }

    fn dispatch_cancel(
        &mut self,
        request_id: RequestId,
        target_request_id: RequestId,
    ) -> Result<SessionUpdate, SessionError> {
        if request_id == target_request_id {
            return self.rejected_update(request_id, RejectionReason::UnknownCancellationTarget);
        }
        let preview = self.requests.cancellation_outcome(target_request_id);
        if preview == CancellationOutcome::UnknownRequest {
            return self.rejected_update(request_id, RejectionReason::UnknownCancellationTarget);
        }
        let outcome = self.requests.cancel(target_request_id);
        let target_is_pending = self
            .pending
            .is_some_and(|pending| pending.token().request_id() == target_request_id);
        if target_is_pending && outcome == CancellationOutcome::CancelledBeforeCommit {
            self.pending = None;
            self.bump_buffer_generation()?;
            self.active_token = None;
            self.state = SessionState::Stopped;
            self.bump_state_version()?;
        }
        self.record_applied_request(request_id)?;
        Ok(self.session_update(
            CommandResult::new(
                request_id,
                ResultDisposition::Cancelled {
                    target_request_id,
                    outcome,
                },
            ),
            None,
        ))
    }

    fn apply_source_ready(
        &mut self,
        pending: PendingOperation,
    ) -> Result<WorkerUpdate, SessionError> {
        let PendingOperation::Play { token, .. } = pending else {
            return Ok(self.worker_update(
                ResultDisposition::Rejected {
                    reason: RejectionReason::UnexpectedWorkerEvent,
                },
                None,
            ));
        };
        let Some(item_id) = self.current_item.clone() else {
            return Ok(self.worker_update(
                ResultDisposition::Rejected {
                    reason: RejectionReason::NoCurrentItem,
                },
                None,
            ));
        };
        self.state = SessionState::Buffering;
        self.bump_state_version()?;
        Ok(self.worker_update(
            ResultDisposition::Accepted {
                revision: token.revision(),
            },
            Some(WorkerIntent::PrepareBuffer {
                token,
                item_id,
                position_us: self.clock.media_position_us(),
            }),
        ))
    }

    fn apply_prebuffer_ready(
        &mut self,
        pending: PendingOperation,
    ) -> Result<WorkerUpdate, SessionError> {
        let (token, next_state, boundary) = match pending {
            PendingOperation::Play {
                token,
                resumes_from_pause: true,
            }
            | PendingOperation::Resume { token } => {
                (token, SessionState::Playing, ActivationBoundary::Resume)
            }
            PendingOperation::Play {
                token,
                resumes_from_pause: false,
            } => (
                token,
                SessionState::Playing,
                ActivationBoundary::SourceTransition,
            ),
            PendingOperation::Seek {
                token,
                prior_state,
                applied: true,
                ..
            } => (
                token,
                if prior_state == SessionState::Paused {
                    SessionState::Paused
                } else {
                    SessionState::Playing
                },
                ActivationBoundary::None,
            ),
            PendingOperation::Seek { .. } => {
                return Ok(self.worker_update(
                    ResultDisposition::Rejected {
                        reason: RejectionReason::UnexpectedWorkerEvent,
                    },
                    None,
                ));
            }
        };
        match boundary {
            ActivationBoundary::None => {}
            ActivationBoundary::Resume => self.clock.resume_boundary()?,
            ActivationBoundary::SourceTransition => self.clock.source_transition(0)?,
        }
        self.state = next_state;
        self.pending = None;
        let applied = self.requests.mark_applied(token.request_id());
        debug_assert!(applied);
        self.active_token = Some(token);
        self.bump_state_version()?;
        Ok(self.worker_update(
            ResultDisposition::Accepted {
                revision: token.revision(),
            },
            None,
        ))
    }

    fn apply_seek_applied(
        &mut self,
        pending: PendingOperation,
        position_us: u64,
    ) -> Result<WorkerUpdate, SessionError> {
        let PendingOperation::Seek {
            token,
            position_us: expected_position,
            prior_state,
            ..
        } = pending
        else {
            return Ok(self.worker_update(
                ResultDisposition::Rejected {
                    reason: RejectionReason::UnexpectedWorkerEvent,
                },
                None,
            ));
        };
        if position_us != expected_position {
            return Ok(self.worker_update(
                ResultDisposition::Stale {
                    reason: StaleReason::PayloadMismatch,
                },
                None,
            ));
        }
        self.clock.seek(position_us)?;
        self.pending = Some(PendingOperation::Seek {
            token,
            position_us: expected_position,
            prior_state,
            applied: true,
        });
        if prior_state != SessionState::Paused {
            self.state = SessionState::Buffering;
        }
        self.bump_state_version()?;
        let item_id = self.current_item.clone();
        Ok(self.worker_update(
            ResultDisposition::Accepted {
                revision: token.revision(),
            },
            item_id.map(|item_id| WorkerIntent::PrepareBuffer {
                token,
                item_id,
                position_us,
            }),
        ))
    }

    fn apply_terminal_worker_event(
        &mut self,
        pending: PendingOperation,
        state: SessionState,
    ) -> Result<WorkerUpdate, SessionError> {
        let token = pending.token();
        self.pending = None;
        self.active_token = None;
        self.state = state;
        let applied = self.requests.mark_applied(token.request_id());
        debug_assert!(applied);
        self.bump_state_version()?;
        Ok(self.worker_update(
            ResultDisposition::Accepted {
                revision: token.revision(),
            },
            None,
        ))
    }

    fn apply_active_terminal_event(
        &mut self,
        token: OperationToken,
        state: SessionState,
    ) -> Result<WorkerUpdate, SessionError> {
        self.active_token = None;
        self.state = state;
        self.bump_state_version()?;
        Ok(self.worker_update(
            ResultDisposition::Accepted {
                revision: token.revision(),
            },
            None,
        ))
    }

    fn start_request(&mut self, request_id: RequestId) -> Result<OperationToken, SessionError> {
        self.requests
            .start(request_id, self.buffer_generation)
            .map_err(SessionError::from)
    }

    fn record_applied_request(
        &mut self,
        request_id: RequestId,
    ) -> Result<OperationToken, SessionError> {
        self.requests
            .record_applied(request_id, self.buffer_generation)
            .map_err(SessionError::from)
    }

    fn rejected_update(
        &mut self,
        request_id: RequestId,
        reason: RejectionReason,
    ) -> Result<SessionUpdate, SessionError> {
        self.requests
            .record_rejected(request_id)
            .map_err(SessionError::from)?;
        Ok(self.session_update(
            CommandResult::new(request_id, ResultDisposition::Rejected { reason }),
            None,
        ))
    }

    fn session_update(&self, result: CommandResult, intent: Option<WorkerIntent>) -> SessionUpdate {
        SessionUpdate {
            result,
            intent,
            snapshot: self.make_snapshot(),
        }
    }

    fn worker_update(
        &self,
        disposition: ResultDisposition,
        intent: Option<WorkerIntent>,
    ) -> WorkerUpdate {
        WorkerUpdate {
            disposition,
            intent,
            snapshot: self.make_snapshot(),
        }
    }

    fn make_snapshot(&self) -> PlaybackSnapshot {
        PlaybackSnapshot {
            state: self.state,
            current_item: self.current_item.clone(),
            clock: self.clock,
            state_version: self.state_version,
            buffer_generation: self.buffer_generation,
            pending_request_id: self.pending.map(|pending| pending.token().request_id()),
        }
    }

    fn bump_state_version(&mut self) -> Result<(), SessionError> {
        self.state_version = self
            .state_version
            .checked_add(1)
            .ok_or(SessionError::StateVersionExhausted)?;
        Ok(())
    }

    fn bump_buffer_generation(&mut self) -> Result<(), SessionError> {
        self.buffer_generation = self
            .buffer_generation
            .checked_add(1)
            .ok_or(SessionError::BufferGenerationExhausted)?;
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ActivationBoundary {
    None,
    Resume,
    SourceTransition,
}
