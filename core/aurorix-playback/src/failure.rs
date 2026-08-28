//! Deterministic playback failure policy.
//!
//! The policy returns control-plane decisions only. It never edits catalog,
//! playlist, favorite, or history storage, and it never carries a runtime path,
//! file handle, URL, credential, or provider lease.

use std::{error::Error, fmt};

use aurorix_model::play_fact::EndReason;

use crate::command::PlaybackItemId;

/// A failure class safe to carry across the playback control boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PlaybackFailure {
    /// The local source disappeared or could not be opened.
    SourceLoss,
    /// The selected decoder failed on the current item.
    DecoderError,
    /// The output path became unavailable.
    OutputUnavailable,
    /// The user or queue explicitly skipped the current item.
    Skip,
    /// The user explicitly stopped the current item.
    Stop,
}

/// The queue action selected by a failure decision.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QueueAction {
    /// The current item is finalized and the next queue item may be selected.
    Continue {
        /// Stable Core identity of the next item.
        next_item: PlaybackItemId,
        /// Stable position in the queue supplied to the policy.
        next_index: usize,
    },
    /// The current item is finalized and no next item is selected.
    Stop {
        /// Whether the queue ended while looking for a continuation.
        queue_exhausted: bool,
    },
}

impl QueueAction {
    /// Reports whether this decision selects another queue item.
    #[must_use]
    pub const fn continues_queue(&self) -> bool {
        matches!(self, Self::Continue { .. })
    }
}

/// An explicit statement that runtime failure does not delete durable identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct IdentityRetention {
    preserved: bool,
}

impl IdentityRetention {
    /// Returns the retention contract for every playback failure decision.
    #[must_use]
    pub const fn preserved() -> Self {
        Self { preserved: true }
    }

    /// Returns whether the local catalog identity remains intact.
    #[must_use]
    pub const fn catalog(self) -> bool {
        self.preserved
    }

    /// Returns whether playlist references remain intact.
    #[must_use]
    pub const fn playlist(self) -> bool {
        self.preserved
    }

    /// Returns whether favorites remain intact.
    #[must_use]
    pub const fn favorite(self) -> bool {
        self.preserved
    }

    /// Returns whether play-history identity remains intact.
    #[must_use]
    pub const fn history(self) -> bool {
        self.preserved
    }
}

/// One immutable control-plane decision for the failed item.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FailureDecision {
    failed_item: PlaybackItemId,
    failure: PlaybackFailure,
    end_reason: EndReason,
    queue_action: QueueAction,
    identity_retention: IdentityRetention,
}

impl FailureDecision {
    /// Returns the failed item's Core identity.
    #[must_use]
    pub fn failed_item(&self) -> &PlaybackItemId {
        &self.failed_item
    }

    /// Returns the classified failure.
    #[must_use]
    pub const fn failure(&self) -> PlaybackFailure {
        self.failure
    }

    /// Returns the existing immutable fact end reason to use at finalization.
    #[must_use]
    pub const fn end_reason(&self) -> EndReason {
        self.end_reason
    }

    /// Returns the queue continuation decision.
    #[must_use]
    pub fn queue_action(&self) -> &QueueAction {
        &self.queue_action
    }

    /// Returns the identity-preservation contract.
    #[must_use]
    pub const fn identity_retention(&self) -> IdentityRetention {
        self.identity_retention
    }
}

/// Policy knobs for failure continuation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FailurePolicy {
    continue_on_source_loss: bool,
    continue_on_decoder_error: bool,
    continue_on_output_unavailable: bool,
}

impl FailurePolicy {
    /// Creates a policy with explicit continuation behavior for each failure.
    #[must_use]
    pub const fn new(
        continue_on_source_loss: bool,
        continue_on_decoder_error: bool,
        continue_on_output_unavailable: bool,
    ) -> Self {
        Self {
            continue_on_source_loss,
            continue_on_decoder_error,
            continue_on_output_unavailable,
        }
    }

    /// The default local policy skips a failed source/decoder item but stops
    /// when the output path itself is unavailable.
    #[must_use]
    pub const fn default_local() -> Self {
        Self::new(true, true, false)
    }

    /// Returns whether this policy continues after the supplied failure.
    #[must_use]
    pub const fn continues_after(self, failure: PlaybackFailure) -> bool {
        match failure {
            PlaybackFailure::SourceLoss => self.continue_on_source_loss,
            PlaybackFailure::DecoderError => self.continue_on_decoder_error,
            PlaybackFailure::OutputUnavailable => self.continue_on_output_unavailable,
            PlaybackFailure::Skip => true,
            PlaybackFailure::Stop => false,
        }
    }

    /// Applies a source/decoder/output failure to a stable queue snapshot.
    ///
    /// `current_index` is checked against both the queue and `current_item` so
    /// a stale queue view cannot select the wrong identity.
    ///
    /// # Errors
    ///
    /// Returns a typed queue error when the supplied current item is not the
    /// item at the supplied queue position.
    pub fn decide(
        self,
        failure: PlaybackFailure,
        current_item: &PlaybackItemId,
        queue: &[PlaybackItemId],
        current_index: usize,
    ) -> Result<FailureDecision, FailurePolicyError> {
        validate_current_item(current_item, queue, current_index)?;
        let queue_action = if self.continues_after(failure) {
            next_action(queue, current_index)
        } else {
            QueueAction::Stop {
                queue_exhausted: false,
            }
        };
        Ok(FailureDecision {
            failed_item: current_item.clone(),
            failure,
            end_reason: EndReason::Error,
            queue_action,
            identity_retention: IdentityRetention::preserved(),
        })
    }

    /// Finalizes the current item as skipped and selects the next item when it
    /// exists. This is the deterministic `next`/skip policy used by the queue.
    ///
    /// # Errors
    ///
    /// Returns a typed queue error for a stale current-item position.
    pub fn skip(
        self,
        current_item: &PlaybackItemId,
        queue: &[PlaybackItemId],
        current_index: usize,
    ) -> Result<FailureDecision, FailurePolicyError> {
        validate_current_item(current_item, queue, current_index)?;
        debug_assert!(self.continues_after(PlaybackFailure::Skip));
        Ok(FailureDecision {
            failed_item: current_item.clone(),
            failure: PlaybackFailure::Skip,
            end_reason: EndReason::Skipped,
            queue_action: next_action(queue, current_index),
            identity_retention: IdentityRetention::preserved(),
        })
    }

    /// Finalizes the current item as explicitly stopped without continuation.
    ///
    /// # Errors
    ///
    /// Returns a typed queue error for a stale current-item position.
    pub fn stop(
        self,
        current_item: &PlaybackItemId,
        queue: &[PlaybackItemId],
        current_index: usize,
    ) -> Result<FailureDecision, FailurePolicyError> {
        validate_current_item(current_item, queue, current_index)?;
        debug_assert!(!self.continues_after(PlaybackFailure::Stop));
        Ok(FailureDecision {
            failed_item: current_item.clone(),
            failure: PlaybackFailure::Stop,
            end_reason: EndReason::Stopped,
            queue_action: QueueAction::Stop {
                queue_exhausted: false,
            },
            identity_retention: IdentityRetention::preserved(),
        })
    }
}

impl Default for FailurePolicy {
    fn default() -> Self {
        Self::default_local()
    }
}

/// Validation failures for a queue snapshot supplied to the policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailurePolicyError {
    /// No queue item exists at the supplied current index.
    CurrentIndexOutOfBounds,
    /// The current item does not match the queue item at that index.
    CurrentItemMismatch,
}

impl fmt::Display for FailurePolicyError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::CurrentIndexOutOfBounds => {
                formatter.write_str("current queue index is out of bounds")
            }
            Self::CurrentItemMismatch => {
                formatter.write_str("current item does not match the queue snapshot")
            }
        }
    }
}

impl Error for FailurePolicyError {}

fn validate_current_item(
    current_item: &PlaybackItemId,
    queue: &[PlaybackItemId],
    current_index: usize,
) -> Result<(), FailurePolicyError> {
    let queued_item = queue
        .get(current_index)
        .ok_or(FailurePolicyError::CurrentIndexOutOfBounds)?;
    if queued_item != current_item {
        return Err(FailurePolicyError::CurrentItemMismatch);
    }
    Ok(())
}

fn next_action(queue: &[PlaybackItemId], current_index: usize) -> QueueAction {
    queue.get(current_index + 1).map_or(
        QueueAction::Stop {
            queue_exhausted: true,
        },
        |next_item| QueueAction::Continue {
            next_item: next_item.clone(),
            next_index: current_index + 1,
        },
    )
}

#[cfg(test)]
mod tests {
    use super::{FailurePolicy, PlaybackFailure, QueueAction};
    use crate::command::PlaybackItemId;

    fn item(value: &str) -> PlaybackItemId {
        PlaybackItemId::new(value).expect("valid item identity")
    }

    fn queue() -> Vec<PlaybackItemId> {
        vec![item("one"), item("two"), item("three")]
    }

    #[test]
    fn source_loss_continues_without_deleting_any_identity() {
        let queue = queue();
        let decision = FailurePolicy::default_local()
            .decide(PlaybackFailure::SourceLoss, &queue[0], &queue, 0)
            .expect("queue position is current");
        assert_eq!(decision.end_reason().as_str(), "error");
        assert_eq!(decision.failed_item().as_str(), "one");
        assert_eq!(
            decision.queue_action(),
            &QueueAction::Continue {
                next_item: item("two"),
                next_index: 1,
            }
        );
        let retention = decision.identity_retention();
        assert!(retention.catalog());
        assert!(retention.playlist());
        assert!(retention.favorite());
        assert!(retention.history());
    }

    #[test]
    fn decoder_error_can_be_configured_to_stop() {
        let queue = queue();
        let decision = FailurePolicy::new(true, false, false)
            .decide(PlaybackFailure::DecoderError, &queue[1], &queue, 1)
            .expect("queue position is current");
        assert_eq!(decision.failure(), PlaybackFailure::DecoderError);
        assert_eq!(
            decision.queue_action(),
            &QueueAction::Stop {
                queue_exhausted: false
            }
        );
    }

    #[test]
    fn skip_and_stop_have_distinct_terminal_reasons() {
        let queue = queue();
        let skipped = FailurePolicy::default()
            .skip(&queue[0], &queue, 0)
            .expect("queue position is current");
        assert_eq!(skipped.end_reason().as_str(), "skipped");
        assert!(skipped.queue_action().continues_queue());

        let stopped = FailurePolicy::default()
            .stop(&queue[0], &queue, 0)
            .expect("queue position is current");
        assert_eq!(stopped.failure(), PlaybackFailure::Stop);
        assert_eq!(stopped.end_reason().as_str(), "stopped");
        assert!(!stopped.queue_action().continues_queue());
    }

    #[test]
    fn queue_end_is_a_deterministic_stop_and_stale_positions_are_rejected() {
        let queue = queue();
        let decision = FailurePolicy::default()
            .decide(PlaybackFailure::SourceLoss, &queue[2], &queue, 2)
            .expect("queue position is current");
        assert_eq!(
            decision.queue_action(),
            &QueueAction::Stop {
                queue_exhausted: true
            }
        );
        assert!(
            FailurePolicy::default()
                .decide(PlaybackFailure::SourceLoss, &queue[0], &queue, 1)
                .is_err()
        );
    }
}
