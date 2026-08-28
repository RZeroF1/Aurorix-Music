//! Deterministic Gate 2A contract vectors.

#[cfg(test)]
mod tests {
    use crate::{
        clock::{ClockError, DiscontinuityReason, PlaybackRate, PresentationClock},
        command::{
            CancellationOutcome, PlaybackAction, PlaybackCommand, PlaybackItemId, RequestId,
            RequestTracker, ResultDisposition, StaleReason,
        },
        session::{PlaybackSession, SessionState, WorkerEvent, WorkerIntent},
    };

    fn item(value: &str) -> PlaybackItemId {
        PlaybackItemId::new(value).expect("fixture item ID")
    }

    fn command(request_id: u64, action: PlaybackAction) -> PlaybackCommand {
        PlaybackCommand::new(RequestId::new(request_id), action)
    }

    #[test]
    fn request_tracker_classifies_cancelled_and_superseded_results() {
        let mut tracker = RequestTracker::new();
        let first = tracker.start(RequestId::new(1), 0).expect("first request");
        let second = tracker.start(RequestId::new(2), 0).expect("second request");

        assert_eq!(
            tracker.classify(first, 0),
            ResultDisposition::Stale {
                reason: StaleReason::RequestSuperseded
            }
        );
        assert_eq!(
            tracker.cancel(RequestId::new(2)),
            CancellationOutcome::CancelledBeforeCommit
        );
        assert_eq!(
            tracker.classify(second, 0),
            ResultDisposition::Cancelled {
                target_request_id: RequestId::new(2),
                outcome: CancellationOutcome::CancelledBeforeCommit,
            }
        );
        assert_eq!(
            tracker.classify(first.with_buffer_generation(1), 1),
            ResultDisposition::Stale {
                reason: StaleReason::RequestSuperseded
            }
        );
    }

    #[test]
    fn clock_uses_rendered_frames_and_exposes_boundaries() {
        let mut clock = PresentationClock::new(48_000).expect("valid sample rate");
        clock
            .advance_rendered_frames(4_800)
            .expect("ten percent second");
        assert_eq!(clock.rendered_frames(), 4_800);
        assert_eq!(clock.media_position_us(), 100_000);
        assert_eq!(clock.clock_epoch(), 0);

        clock.pause_boundary().expect("pause boundary");
        assert_eq!(clock.clock_epoch(), 1);
        assert_eq!(clock.media_position_us(), 100_000);
        assert_eq!(
            clock.discontinuity_reason(),
            Some(DiscontinuityReason::Pause)
        );
        clock.acknowledge_discontinuity();
        assert!(!clock.is_discontinuous());
        clock.resume_boundary().expect("resume boundary");
        assert_eq!(clock.clock_epoch(), 2);
        clock
            .set_playback_rate(PlaybackRate::from_millionths(500_000).unwrap())
            .expect("half speed");
        clock.advance_rendered_frames(4_800).expect("half second");
        assert_eq!(clock.media_position_us(), 150_000);
    }

    #[test]
    fn clock_rejects_invalid_configuration_without_panicking() {
        assert_eq!(
            PresentationClock::new(0),
            Err(ClockError::InvalidSampleRate)
        );
        assert_eq!(
            PlaybackRate::from_millionths(0),
            Err(ClockError::InvalidPlaybackRate)
        );
    }

    #[test]
    fn clock_rejects_position_overflow_without_partial_mutation() {
        let mut clock = PresentationClock::new(1).expect("valid sample rate");
        assert_eq!(
            clock.advance_rendered_frames(u64::MAX),
            Err(ClockError::PositionOverflow)
        );
        assert_eq!(clock.rendered_frames(), 0);
        assert_eq!(clock.media_position_us(), 0);
    }

    #[test]
    fn session_play_pause_resume_and_rendering_are_deterministic() {
        let mut session = PlaybackSession::new(48_000).expect("valid session");
        let play = session
            .dispatch(command(
                1,
                PlaybackAction::Play {
                    item_id: Some(item("track-1")),
                },
            ))
            .expect("play command");
        assert_eq!(play.snapshot().state(), SessionState::Loading);
        assert!(matches!(
            play.intent(),
            Some(WorkerIntent::ResolveSource { .. })
        ));
        assert_eq!(play.snapshot().clock().clock_epoch(), 0);
        let token = play.intent().expect("resolve intent").token();

        session
            .handle_worker_event(WorkerEvent::SourceReady { token })
            .expect("source ready");
        let prebuffer = session
            .handle_worker_event(WorkerEvent::PrebufferReady { token })
            .expect("prebuffer ready");
        assert_eq!(prebuffer.snapshot().state(), SessionState::Playing);
        assert_eq!(
            prebuffer.snapshot().clock().discontinuity_reason(),
            Some(DiscontinuityReason::SourceTransition)
        );
        session
            .record_rendered_frames(4_800)
            .expect("rendered frames");
        let position = session.clock().media_position_us();
        assert_eq!(position, 100_000);

        let pause = session
            .dispatch(command(2, PlaybackAction::Pause))
            .expect("pause command");
        assert_eq!(pause.snapshot().state(), SessionState::Paused);
        session
            .record_rendered_frames(4_800)
            .expect("ignored paused frames");
        assert_eq!(session.clock().media_position_us(), position);

        let resume = session
            .dispatch(command(3, PlaybackAction::Resume))
            .expect("resume command");
        assert_eq!(resume.snapshot().state(), SessionState::Buffering);
        let resume_epoch = resume.snapshot().clock().clock_epoch();
        let resume_token = resume.intent().expect("resume intent").token();
        let resumed = session
            .handle_worker_event(WorkerEvent::PrebufferReady {
                token: resume_token,
            })
            .expect("resume prebuffer");
        assert_eq!(session.state(), SessionState::Playing);
        assert_eq!(resumed.snapshot().clock().clock_epoch(), resume_epoch + 1);
        assert_eq!(
            resumed.snapshot().clock().discontinuity_reason(),
            Some(DiscontinuityReason::Resume)
        );
    }

    #[test]
    fn identical_control_and_worker_sequences_produce_identical_snapshots() {
        fn replay() -> crate::session::PlaybackSnapshot {
            let mut session = PlaybackSession::new(48_000).expect("valid session");
            let play = session
                .dispatch(command(
                    10,
                    PlaybackAction::Play {
                        item_id: Some(item("track-1")),
                    },
                ))
                .expect("play command");
            let token = play.intent().expect("resolve intent").token();
            session
                .handle_worker_event(WorkerEvent::SourceReady { token })
                .expect("source ready");
            session
                .handle_worker_event(WorkerEvent::PrebufferReady { token })
                .expect("prebuffer ready");
            session
                .record_rendered_frames(9_600)
                .expect("rendered frames");
            session
                .dispatch(command(11, PlaybackAction::Pause))
                .expect("pause command");
            session.snapshot()
        }

        assert_eq!(replay(), replay());
    }

    #[test]
    fn seek_invalidates_old_generation_and_keeps_pause_semantics() {
        let mut session = PlaybackSession::new(48_000).expect("valid session");
        let play = session
            .dispatch(command(
                1,
                PlaybackAction::Play {
                    item_id: Some(item("track-1")),
                },
            ))
            .expect("play command");
        let token = play.intent().expect("resolve intent").token();
        session
            .handle_worker_event(WorkerEvent::SourceReady { token })
            .expect("source ready");
        session
            .handle_worker_event(WorkerEvent::PrebufferReady { token })
            .expect("prebuffer ready");

        let seek = session
            .dispatch(command(
                2,
                PlaybackAction::Seek {
                    position_us: 250_000,
                },
            ))
            .expect("seek command");
        let seek_token = seek.intent().expect("seek intent").token();
        let stale = session
            .handle_worker_event(WorkerEvent::PrebufferReady { token })
            .expect("stale old result");
        assert_eq!(
            stale.disposition(),
            ResultDisposition::Stale {
                reason: StaleReason::BufferGenerationChanged
            }
        );
        let applied = session
            .handle_worker_event(WorkerEvent::SeekApplied {
                token: seek_token,
                position_us: 250_000,
            })
            .expect("seek applied");
        assert!(applied.snapshot().clock().is_discontinuous());
        session
            .handle_worker_event(WorkerEvent::PrebufferReady { token: seek_token })
            .expect("seek prebuffer");
        assert_eq!(session.state(), SessionState::Playing);
        assert_eq!(session.clock().media_position_us(), 250_000);
    }

    #[test]
    fn seek_while_paused_keeps_the_session_paused_after_prebuffering() {
        let mut session = PlaybackSession::new(48_000).expect("valid session");
        let play = session
            .dispatch(command(
                1,
                PlaybackAction::Play {
                    item_id: Some(item("track-1")),
                },
            ))
            .expect("play command");
        let token = play.intent().expect("resolve intent").token();
        session
            .handle_worker_event(WorkerEvent::SourceReady { token })
            .expect("source ready");
        session
            .handle_worker_event(WorkerEvent::PrebufferReady { token })
            .expect("prebuffer ready");
        session
            .dispatch(command(2, PlaybackAction::Pause))
            .expect("pause command");

        let seek = session
            .dispatch(command(
                3,
                PlaybackAction::Seek {
                    position_us: 75_000,
                },
            ))
            .expect("seek command");
        let seek_token = seek.intent().expect("seek intent").token();
        session
            .handle_worker_event(WorkerEvent::SeekApplied {
                token: seek_token,
                position_us: 75_000,
            })
            .expect("seek applied");
        session
            .handle_worker_event(WorkerEvent::PrebufferReady { token: seek_token })
            .expect("prebuffer ready");

        assert_eq!(session.state(), SessionState::Paused);
        assert_eq!(session.clock().media_position_us(), 75_000);
    }

    #[test]
    fn active_playback_can_report_a_terminal_worker_outcome() {
        let mut session = PlaybackSession::new(48_000).expect("valid session");
        let play = session
            .dispatch(command(
                1,
                PlaybackAction::Play {
                    item_id: Some(item("track-1")),
                },
            ))
            .expect("play command");
        let token = play.intent().expect("resolve intent").token();
        session
            .handle_worker_event(WorkerEvent::SourceReady { token })
            .expect("source ready");
        session
            .handle_worker_event(WorkerEvent::PrebufferReady { token })
            .expect("prebuffer ready");

        let ended = session
            .handle_worker_event(WorkerEvent::Ended { token })
            .expect("ended event");
        assert_eq!(ended.snapshot().state(), SessionState::Ended);
        assert!(ended.snapshot().pending_request_id().is_none());
    }

    #[test]
    fn cancellation_does_not_accept_a_late_worker_result() {
        let mut session = PlaybackSession::new(48_000).expect("valid session");
        let play = session
            .dispatch(command(
                1,
                PlaybackAction::Play {
                    item_id: Some(item("track-1")),
                },
            ))
            .expect("play command");
        let token = play.intent().expect("resolve intent").token();
        let cancel = session
            .dispatch(command(
                2,
                PlaybackAction::Cancel {
                    target_request_id: RequestId::new(1),
                },
            ))
            .expect("cancel command");
        assert_eq!(
            cancel.result().disposition(),
            ResultDisposition::Cancelled {
                target_request_id: RequestId::new(1),
                outcome: CancellationOutcome::CancelledBeforeCommit,
            }
        );
        let late = session
            .handle_worker_event(WorkerEvent::SourceReady { token })
            .expect("late event classification");
        assert!(matches!(
            late.disposition(),
            ResultDisposition::Cancelled { .. }
        ));
        assert_eq!(late.snapshot().state(), SessionState::Stopped);
    }

    #[test]
    fn duplicate_commands_are_rejected_without_a_second_transition() {
        let mut session = PlaybackSession::new(48_000).expect("valid session");
        let first = session
            .dispatch(command(
                1,
                PlaybackAction::Play {
                    item_id: Some(item("track-1")),
                },
            ))
            .expect("first command");
        let version = first.snapshot().state_version();
        let duplicate = session
            .dispatch(command(
                1,
                PlaybackAction::Play {
                    item_id: Some(item("track-2")),
                },
            ))
            .expect("duplicate command");
        assert_eq!(
            duplicate.result().disposition(),
            ResultDisposition::Rejected {
                reason: crate::command::RejectionReason::DuplicateRequest
            }
        );
        assert_eq!(duplicate.snapshot().state_version(), version);
        assert_eq!(duplicate.snapshot().current_item(), Some(&item("track-1")));
    }
}
