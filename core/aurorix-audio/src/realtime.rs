//! Callback-facing PCM consumption, compact clocks, and bounded signals.
//!
//! [`RealtimeConsumer::render`] is deliberately limited to prepared SPSC
//! blocks, bounded slice copies, and atomic publication. It does not open a
//! source, decode, allocate, lock, perform I/O, wait for a Worker, format a
//! log message, or call a platform/UI/Provider interface.

use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicU8, AtomicU32, AtomicU64, Ordering},
};

use super::buffer::{AudioBuffer, ReadyBlock};

const NO_FAULT: u8 = 0;

/// A compact, platform-neutral clock observation written by the callback.
///
/// This is intentionally raw renderer state rather than a replacement for
/// `aurorix-playback`'s `PresentationClock`. The playback facade translates
/// it into the shared public projection, including media microseconds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CompactClockSample {
    /// Session-controlled discontinuity epoch.
    pub clock_epoch: u64,
    /// Total output frames presented since the active timeline began.
    pub rendered_frames: u64,
    /// Source-media frames actually consumed from prepared PCM.
    pub media_position_frames: u64,
    /// Output sample rate used to interpret frame counts.
    pub output_sample_rate_hz: u32,
    /// Current estimated downstream output latency.
    pub estimated_output_latency_frames: u32,
}

impl CompactClockSample {
    /// Creates the first sample for a presentation timeline.
    #[must_use]
    pub const fn new(
        clock_epoch: u64,
        media_position_frames: u64,
        output_sample_rate_hz: u32,
        estimated_output_latency_frames: u32,
    ) -> Self {
        Self {
            clock_epoch,
            rendered_frames: 0,
            media_position_frames,
            output_sample_rate_hz,
            estimated_output_latency_frames,
        }
    }
}

/// A single-writer, multi-reader latest-value publisher for compact clocks.
///
/// The realtime callback is the only writer. Readers use [`Self::try_latest`]
/// to obtain a coherent snapshot without taking a lock or subscribing to one
/// event per callback.
#[derive(Debug)]
pub struct CompactClockPublisher {
    sequence: AtomicU64,
    clock_epoch: AtomicU64,
    rendered_frames: AtomicU64,
    media_position_frames: AtomicU64,
    output_sample_rate_hz: AtomicU32,
    estimated_output_latency_frames: AtomicU32,
}

impl CompactClockPublisher {
    /// Creates a publisher with an initial clock sample.
    #[must_use]
    pub const fn new(initial: CompactClockSample) -> Self {
        Self {
            sequence: AtomicU64::new(0),
            clock_epoch: AtomicU64::new(initial.clock_epoch),
            rendered_frames: AtomicU64::new(initial.rendered_frames),
            media_position_frames: AtomicU64::new(initial.media_position_frames),
            output_sample_rate_hz: AtomicU32::new(initial.output_sample_rate_hz),
            estimated_output_latency_frames: AtomicU32::new(
                initial.estimated_output_latency_frames,
            ),
        }
    }

    /// Publishes one callback observation.
    ///
    /// This operation has a single callback writer. It performs only atomic
    /// stores and never waits for a reader.
    pub fn publish(&self, sample: CompactClockSample) {
        self.sequence.fetch_add(1, Ordering::AcqRel);
        self.clock_epoch
            .store(sample.clock_epoch, Ordering::Relaxed);
        self.rendered_frames
            .store(sample.rendered_frames, Ordering::Relaxed);
        self.media_position_frames
            .store(sample.media_position_frames, Ordering::Relaxed);
        self.output_sample_rate_hz
            .store(sample.output_sample_rate_hz, Ordering::Relaxed);
        self.estimated_output_latency_frames
            .store(sample.estimated_output_latency_frames, Ordering::Relaxed);
        self.sequence.fetch_add(1, Ordering::Release);
    }

    /// Attempts to read one coherent latest-value snapshot.
    ///
    /// `None` means the callback was publishing while this reader sampled. A
    /// UI or coordinator can retry later; it must not force callback work.
    #[must_use]
    pub fn try_latest(&self) -> Option<CompactClockSample> {
        let before = self.sequence.load(Ordering::Acquire);
        if !before.is_multiple_of(2) {
            return None;
        }
        let sample = CompactClockSample {
            clock_epoch: self.clock_epoch.load(Ordering::Relaxed),
            rendered_frames: self.rendered_frames.load(Ordering::Relaxed),
            media_position_frames: self.media_position_frames.load(Ordering::Relaxed),
            output_sample_rate_hz: self.output_sample_rate_hz.load(Ordering::Relaxed),
            estimated_output_latency_frames: self
                .estimated_output_latency_frames
                .load(Ordering::Relaxed),
        };
        let after = self.sequence.load(Ordering::Acquire);
        (before == after).then_some(sample)
    }
}

/// A coalesced underrun observation for a non-realtime coordinator.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UnderrunSignal {
    /// Monotonic callback-side signal sequence. It may skip values because
    /// multiple underruns are coalesced before the coordinator samples it.
    pub sequence: u64,
    /// Missing output frames from the most recently observed underrun.
    pub missing_frames: u32,
}

/// A compact fault that can cross the realtime boundary without strings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RealtimeFault {
    /// An output adapter supplied a slice not divisible by the channel count.
    OutputFrameMisalignment,
    /// A block became stale while the callback was selecting ready work.
    StaleBlock,
    /// A Worker reported a decoder failure through the bounded fault cell.
    DecoderFailure,
    /// A Worker reported that the output adapter became unavailable.
    OutputUnavailable,
}

impl RealtimeFault {
    const fn code(self) -> u8 {
        match self {
            Self::OutputFrameMisalignment => 1,
            Self::StaleBlock => 2,
            Self::DecoderFailure => 3,
            Self::OutputUnavailable => 4,
        }
    }

    const fn from_code(code: u8) -> Option<Self> {
        match code {
            1 => Some(Self::OutputFrameMisalignment),
            2 => Some(Self::StaleBlock),
            3 => Some(Self::DecoderFailure),
            4 => Some(Self::OutputUnavailable),
            _ => None,
        }
    }
}

/// Bounded, coalescing signals from callback and Worker paths.
///
/// This is not an event queue. It retains only the latest underrun size and
/// first pending fault, which makes its memory usage fixed and ensures a stall
/// in the coordinator cannot grow callback-side state.
#[derive(Debug)]
pub struct RealtimeSignals {
    underrun_pending: AtomicBool,
    underrun_sequence: AtomicU64,
    latest_missing_frames: AtomicU32,
    fault: AtomicU8,
}

impl RealtimeSignals {
    /// Creates an empty bounded signal set.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            underrun_pending: AtomicBool::new(false),
            underrun_sequence: AtomicU64::new(0),
            latest_missing_frames: AtomicU32::new(0),
            fault: AtomicU8::new(NO_FAULT),
        }
    }

    /// Coalesces an underrun without allocating or waiting for a coordinator.
    pub fn publish_underrun(&self, missing_frames: u32) {
        self.latest_missing_frames
            .store(missing_frames, Ordering::Relaxed);
        self.underrun_sequence.fetch_add(1, Ordering::Relaxed);
        self.underrun_pending.store(true, Ordering::Release);
    }

    /// Records the first pending compact fault.
    ///
    /// A later fault does not overwrite a pending one, so the bounded cell
    /// preserves the first cause that a coordinator still has not consumed.
    pub fn publish_fault(&self, fault: RealtimeFault) {
        let _ = self.fault.compare_exchange(
            NO_FAULT,
            fault.code(),
            Ordering::AcqRel,
            Ordering::Acquire,
        );
    }

    /// Takes one coalesced underrun observation, if any.
    #[must_use]
    pub fn take_underrun(&self) -> Option<UnderrunSignal> {
        self.underrun_pending
            .swap(false, Ordering::AcqRel)
            .then(|| UnderrunSignal {
                sequence: self.underrun_sequence.load(Ordering::Acquire),
                missing_frames: self.latest_missing_frames.load(Ordering::Acquire),
            })
    }

    /// Takes the first pending compact fault, if any.
    #[must_use]
    pub fn take_fault(&self) -> Option<RealtimeFault> {
        RealtimeFault::from_code(self.fault.swap(NO_FAULT, Ordering::AcqRel))
    }
}

impl Default for RealtimeSignals {
    fn default() -> Self {
        Self::new()
    }
}

/// Renderer-owned timeline values for the active local playback item.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RealtimeTimeline {
    /// Current session clock epoch.
    pub clock_epoch: u64,
    /// Media frame offset at the start of this renderer timeline.
    pub media_position_frames: u64,
    /// Adapter-estimated output latency in output frames.
    pub estimated_output_latency_frames: u32,
}

impl RealtimeTimeline {
    /// Creates one renderer timeline.
    #[must_use]
    pub const fn new(
        clock_epoch: u64,
        media_position_frames: u64,
        estimated_output_latency_frames: u32,
    ) -> Self {
        Self {
            clock_epoch,
            media_position_frames,
            estimated_output_latency_frames,
        }
    }
}

/// Result of one bounded callback render request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RenderOutcome {
    /// Complete output frames accepted from the adapter slice.
    pub requested_frames: usize,
    /// Source-media frames copied from prepared blocks.
    pub rendered_media_frames: usize,
    /// Output frames filled with silence because prepared PCM was unavailable.
    pub silent_frames: usize,
    /// Ready blocks discarded because their generation was stale.
    pub discarded_stale_blocks: usize,
    /// Whether the adapter slice ended on a whole interleaved frame.
    pub output_frame_aligned: bool,
}

/// The callback-owned consumer for one [`AudioBuffer`].
///
/// Construct it before the hardware callback starts, then call
/// [`Self::render`] only from that callback's one consumer thread. It owns no
/// mutable Worker state and does not clone its `Arc` fields while rendering.
#[derive(Debug)]
pub struct RealtimeConsumer {
    buffer: Arc<AudioBuffer>,
    clock: Arc<CompactClockPublisher>,
    signals: Arc<RealtimeSignals>,
    next_slot: usize,
    active: Option<ActiveBlock>,
    timeline: RealtimeTimeline,
    rendered_frames: u64,
}

#[derive(Debug, Clone, Copy)]
struct ActiveBlock {
    ready: ReadyBlock,
    consumed_frames: usize,
}

impl RealtimeConsumer {
    /// Creates one callback consumer and its preconfigured observability ports.
    ///
    /// The supplied `Arc` values must be created/cloned before callback
    /// registration. `render` does not clone, drop, or allocate them.
    #[must_use]
    pub fn new(
        buffer: Arc<AudioBuffer>,
        clock: Arc<CompactClockPublisher>,
        signals: Arc<RealtimeSignals>,
        timeline: RealtimeTimeline,
    ) -> Self {
        Self {
            buffer,
            clock,
            signals,
            next_slot: 0,
            active: None,
            timeline,
            rendered_frames: 0,
        }
    }

    /// Returns the current callback-owned timeline.
    #[must_use]
    pub const fn timeline(&self) -> RealtimeTimeline {
        self.timeline
    }

    /// Replaces the timeline after the caller has fenced callback execution.
    ///
    /// This method is a control-plane handoff for seek, restart, or recovery.
    /// It discards a currently held block and must not be called concurrently
    /// with [`Self::render`].
    pub fn reset_timeline(&mut self, timeline: RealtimeTimeline) {
        self.release_active_block();
        self.timeline = timeline;
        self.rendered_frames = 0;
    }

    /// Renders prepared PCM into an adapter-provided interleaved output slice.
    ///
    /// The callback path executes bounded slice copies and atomic operations
    /// only. It never allocates, locks, blocks for input, runs I/O, formats a
    /// log, or invokes a UI, FFI, database, network, or Provider interface.
    #[must_use]
    pub fn render(&mut self, output: &mut [f32]) -> RenderOutcome {
        output.fill(0.0);

        let channels = usize::from(self.buffer.format().channels());
        let requested_frames = output.len() / channels;
        let output_frame_aligned = output.len().is_multiple_of(channels);
        if !output_frame_aligned {
            self.signals
                .publish_fault(RealtimeFault::OutputFrameMisalignment);
            self.publish_clock(0, 0);
            return RenderOutcome {
                requested_frames,
                rendered_media_frames: 0,
                silent_frames: requested_frames,
                discarded_stale_blocks: 0,
                output_frame_aligned: false,
            };
        }

        let mut copied_frames = 0;
        let mut discarded_stale_blocks = 0;
        while copied_frames < requested_frames {
            if self.active.is_none() {
                if !self.try_acquire_active_block(&mut discarded_stale_blocks) {
                    break;
                }
                if self.active.is_none() {
                    continue;
                }
            }

            let Some(mut active) = self.active else {
                continue;
            };
            if active.ready.generation != self.buffer.generation() {
                self.buffer.finish_render(active.ready.index);
                self.active = None;
                discarded_stale_blocks += 1;
                self.signals.publish_fault(RealtimeFault::StaleBlock);
                continue;
            }

            let destination_start = copied_frames * channels;
            let Ok(copied) = self
                .buffer
                .block_for_render(active.ready.index)
                .copy_frames_to(
                    active.consumed_frames,
                    &mut output[destination_start..requested_frames * channels],
                )
            else {
                self.signals
                    .publish_fault(RealtimeFault::OutputFrameMisalignment);
                self.buffer.finish_render(active.ready.index);
                self.active = None;
                break;
            };
            if copied == 0 {
                self.buffer.finish_render(active.ready.index);
                self.active = None;
                continue;
            }

            copied_frames += copied;
            active.consumed_frames += copied;
            if active.consumed_frames >= active.ready.valid_frames {
                self.buffer.finish_render(active.ready.index);
                self.active = None;
            } else {
                self.active = Some(active);
            }
        }

        let silent_frames = requested_frames - copied_frames;
        if silent_frames > 0 {
            self.signals
                .publish_underrun(u32::try_from(silent_frames).unwrap_or(u32::MAX));
        }

        self.publish_clock(requested_frames, copied_frames);

        RenderOutcome {
            requested_frames,
            rendered_media_frames: copied_frames,
            silent_frames,
            discarded_stale_blocks,
            output_frame_aligned,
        }
    }

    fn try_acquire_active_block(&mut self, discarded_stale_blocks: &mut usize) -> bool {
        let slot_index = self.next_slot;
        let Some(ready) = self.buffer.try_begin_render(slot_index) else {
            return false;
        };
        self.next_slot = (self.next_slot + 1) % self.buffer.slot_capacity();
        if ready.generation != self.buffer.generation() {
            self.buffer.finish_render(ready.index);
            *discarded_stale_blocks += 1;
            self.signals.publish_fault(RealtimeFault::StaleBlock);
            return true;
        }
        self.active = Some(ActiveBlock {
            ready,
            consumed_frames: 0,
        });
        true
    }

    fn publish_clock(&mut self, output_frames: usize, media_frames: usize) {
        self.rendered_frames = self
            .rendered_frames
            .saturating_add(u64::try_from(output_frames).unwrap_or(u64::MAX));
        self.timeline.media_position_frames = self
            .timeline
            .media_position_frames
            .saturating_add(u64::try_from(media_frames).unwrap_or(u64::MAX));
        self.clock.publish(CompactClockSample {
            clock_epoch: self.timeline.clock_epoch,
            rendered_frames: self.rendered_frames,
            media_position_frames: self.timeline.media_position_frames,
            output_sample_rate_hz: self.buffer.format().sample_rate_hz(),
            estimated_output_latency_frames: self.timeline.estimated_output_latency_frames,
        });
    }

    fn release_active_block(&mut self) {
        if let Some(active) = self.active.take() {
            self.buffer.finish_render(active.ready.index);
        }
    }
}

impl Drop for RealtimeConsumer {
    fn drop(&mut self) {
        self.release_active_block();
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::{
        CompactClockPublisher, CompactClockSample, RealtimeConsumer, RealtimeFault,
        RealtimeSignals, RealtimeTimeline,
    };
    use crate::{buffer::AudioBuffer, format::PcmFormat};

    fn setup(
        block_frames: usize,
    ) -> (
        Arc<AudioBuffer>,
        Arc<CompactClockPublisher>,
        Arc<RealtimeSignals>,
    ) {
        let buffer = AudioBuffer::new(
            PcmFormat::new(48_000, 2).expect("format is valid"),
            3,
            block_frames,
        )
        .expect("buffer is valid");
        let clock = Arc::new(CompactClockPublisher::new(CompactClockSample::new(
            4, 100, 48_000, 32,
        )));
        let signals = Arc::new(RealtimeSignals::new());
        (buffer, clock, signals)
    }

    fn assert_sample_bits_eq(actual: &[f32], expected: &[f32]) {
        assert_eq!(actual.len(), expected.len());
        assert!(
            actual
                .iter()
                .zip(expected)
                .all(|(actual, expected)| actual.to_bits() == expected.to_bits())
        );
    }

    #[test]
    fn callback_reads_prepared_pcm_and_publishes_compact_clock() {
        let (buffer, clock, signals) = setup(2);
        let mut producer = buffer.producer();
        let lease = producer.try_acquire().expect("slot is vacant");
        lease
            .write_interleaved(&[0.25, -0.25, 0.5, -0.5])
            .expect("samples fit");
        lease.publish().expect("block publishes");

        let mut callback = RealtimeConsumer::new(
            buffer,
            Arc::clone(&clock),
            Arc::clone(&signals),
            RealtimeTimeline::new(4, 100, 32),
        );
        let mut output = [9.0; 4];
        let outcome = callback.render(&mut output);
        assert_eq!(outcome.rendered_media_frames, 2);
        assert_eq!(outcome.silent_frames, 0);
        assert_sample_bits_eq(&output, &[0.25, -0.25, 0.5, -0.5]);
        assert_eq!(signals.take_underrun(), None);
        assert_eq!(
            clock.try_latest(),
            Some(CompactClockSample {
                clock_epoch: 4,
                rendered_frames: 2,
                media_position_frames: 102,
                output_sample_rate_hz: 48_000,
                estimated_output_latency_frames: 32,
            })
        );
    }

    #[test]
    fn callback_outputs_silence_and_coalesces_underrun_when_worker_stalls() {
        let (buffer, clock, signals) = setup(2);
        let mut producer = buffer.producer();
        let _stalled_worker_lease = producer.try_acquire().expect("worker owns a vacant slot");
        let mut callback = RealtimeConsumer::new(
            buffer,
            clock,
            Arc::clone(&signals),
            RealtimeTimeline::new(0, 0, 0),
        );
        let mut output = [5.0; 4];
        let outcome = callback.render(&mut output);
        assert_eq!(outcome.rendered_media_frames, 0);
        assert_eq!(outcome.silent_frames, 2);
        assert_sample_bits_eq(&output, &[0.0; 4]);
        let signal = signals
            .take_underrun()
            .expect("underrun is bounded and visible");
        assert_eq!(signal.missing_frames, 2);
        assert!(signal.sequence >= 1);
    }

    #[test]
    fn callback_keeps_partial_block_for_the_next_output_period() {
        let (buffer, clock, signals) = setup(3);
        let mut producer = buffer.producer();
        let lease = producer.try_acquire().expect("slot is vacant");
        lease
            .write_interleaved(&[1.0, 2.0, 3.0, 4.0, 5.0, 6.0])
            .expect("samples fit");
        lease.publish().expect("block publishes");

        let mut callback = RealtimeConsumer::new(
            buffer,
            Arc::clone(&clock),
            signals,
            RealtimeTimeline::new(2, 10, 0),
        );
        let mut first = [0.0; 4];
        let mut second = [0.0; 4];
        assert_eq!(callback.render(&mut first).rendered_media_frames, 2);
        let second_outcome = callback.render(&mut second);
        assert_sample_bits_eq(&first, &[1.0, 2.0, 3.0, 4.0]);
        assert_sample_bits_eq(&second, &[5.0, 6.0, 0.0, 0.0]);
        assert_eq!(second_outcome.rendered_media_frames, 1);
        assert_eq!(second_outcome.silent_frames, 1);
        assert_eq!(
            clock
                .try_latest()
                .expect("publisher is not busy")
                .media_position_frames,
            13
        );
    }

    #[test]
    fn unaligned_output_is_silenced_and_reported_without_an_error_allocation() {
        let (buffer, clock, signals) = setup(2);
        let mut callback = RealtimeConsumer::new(
            buffer,
            clock,
            Arc::clone(&signals),
            RealtimeTimeline::new(0, 0, 0),
        );
        let mut output = [8.0; 3];
        let outcome = callback.render(&mut output);
        assert!(!outcome.output_frame_aligned);
        assert_sample_bits_eq(&output, &[0.0; 3]);
        assert_eq!(
            signals.take_fault(),
            Some(RealtimeFault::OutputFrameMisalignment)
        );
    }

    #[test]
    fn signals_coalesce_repeated_underruns_and_preserve_first_fault() {
        let signals = RealtimeSignals::new();
        signals.publish_underrun(3);
        signals.publish_underrun(7);
        assert_eq!(
            signals.take_underrun(),
            Some(super::UnderrunSignal {
                sequence: 2,
                missing_frames: 7,
            })
        );
        assert_eq!(signals.take_underrun(), None);

        signals.publish_fault(RealtimeFault::DecoderFailure);
        signals.publish_fault(RealtimeFault::OutputUnavailable);
        assert_eq!(signals.take_fault(), Some(RealtimeFault::DecoderFailure));
        assert_eq!(signals.take_fault(), None);
    }
}
