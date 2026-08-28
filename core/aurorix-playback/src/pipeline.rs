//! Platform-neutral offline playback pipeline.
//!
//! The pipeline is a deterministic control/data-plane harness for local
//! playback. Source resolution, decoder work, seeking, and buffer filling are
//! worker operations. `render_prepared` consumes only already prepared PCM and
//! reports a `PresentationClock` observation to an output sink; it never
//! resolves a source or decodes.
//!
//! The traits in this module are adapter ports. A later audio crate or
//! platform host can implement them without moving a path, handle, URL,
//! credential, or lease into playback state.

use std::{
    collections::{BTreeSet, VecDeque},
    error::Error,
    fmt,
};

use crate::{
    clock::{DiscontinuityReason, PresentationClock},
    command::{OperationToken, PlaybackAction, PlaybackCommand, PlaybackItemId, RequestId},
    queue::{PlaybackQueue, QueueError, QueueSnapshot, QueueTransition},
    session::{
        PlaybackSession, PlaybackSnapshot, SessionError, SessionState, WorkerEvent, WorkerIntent,
    },
};

/// The default output sample rate for an offline pipeline fixture.
pub const DEFAULT_OUTPUT_SAMPLE_RATE_HZ: u32 = 48_000;
/// The default number of channels for an offline pipeline fixture.
pub const DEFAULT_CHANNELS: usize = 2;
/// The default decoder scratch size in frames.
pub const DEFAULT_BLOCK_FRAMES: usize = 1_024;
/// The release-one local prebuffer target in milliseconds.
pub const DEFAULT_PREBUFFER_MS: u32 = 100;

/// One bounded result from a decoder worker step.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct DecodeStep {
    samples_written: usize,
    end_of_stream: bool,
}

impl DecodeStep {
    /// Creates a decoder result.
    #[must_use]
    pub const fn new(samples_written: usize, end_of_stream: bool) -> Self {
        Self {
            samples_written,
            end_of_stream,
        }
    }

    /// Returns the number of interleaved samples written.
    #[must_use]
    pub const fn samples_written(self) -> usize {
        self.samples_written
    }

    /// Returns whether the decoder reached end of stream.
    #[must_use]
    pub const fn end_of_stream(self) -> bool {
        self.end_of_stream
    }
}

/// The effective position returned by a worker-side seek.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SeekStep {
    effective_position_us: u64,
}

impl SeekStep {
    /// Creates a seek result.
    #[must_use]
    pub const fn new(effective_position_us: u64) -> Self {
        Self {
            effective_position_us,
        }
    }

    /// Returns the effective decoder position.
    #[must_use]
    pub const fn effective_position_us(self) -> u64 {
        self.effective_position_us
    }
}

/// A source/decoder adapter owned by a worker plane.
pub trait OfflineDecoder: Send {
    /// Decodes into a caller-provided bounded scratch buffer.
    ///
    /// # Errors
    ///
    /// Returns a typed pipeline error when decoding cannot proceed.
    fn decode(&mut self, output: &mut [f32]) -> Result<DecodeStep, PipelineError>;

    /// Performs a worker-side seek and returns the effective position.
    ///
    /// # Errors
    ///
    /// Returns a typed pipeline error when seeking cannot proceed.
    fn seek(&mut self, position_us: u64) -> Result<SeekStep, PipelineError>;
}

/// The local source resolver and decoder factory.
pub trait OfflineSourceResolver {
    /// Opens a decoder for one Core media identity.
    ///
    /// # Errors
    ///
    /// Returns a typed pipeline error when the source cannot be opened.
    fn open(&mut self, item_id: &PlaybackItemId) -> Result<Box<dyn OfflineDecoder>, PipelineError>;
}

/// A platform-neutral output sink for deterministic capture or test output.
pub trait OfflineOutputSink {
    /// Receives one prepared interleaved output block and clock sample.
    ///
    /// # Errors
    ///
    /// Returns a typed pipeline error when the sink rejects the observation.
    fn write(
        &mut self,
        samples: &[f32],
        clock: PipelineClockSample,
        discontinuity: Option<PipelineDiscontinuity>,
    ) -> Result<(), PipelineError>;
}

/// A compact clock observation emitted to an offline sink.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PipelineClockSample {
    /// Presentation epoch.
    pub clock_epoch: u64,
    /// Output frames rendered by the presentation clock.
    pub rendered_frames: u64,
    /// Media position in microseconds.
    pub media_position_us: u64,
    /// Output sample rate.
    pub output_sample_rate_hz: u32,
    /// Output latency estimate in frames.
    pub estimated_output_latency_frames: u64,
}

impl From<PresentationClock> for PipelineClockSample {
    fn from(clock: PresentationClock) -> Self {
        Self {
            clock_epoch: clock.clock_epoch(),
            rendered_frames: clock.rendered_frames(),
            media_position_us: clock.media_position_us(),
            output_sample_rate_hz: clock.output_sample_rate(),
            estimated_output_latency_frames: clock.estimated_output_latency_frames(),
        }
    }
}

/// A discontinuity marker emitted once until the session acknowledges it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PipelineDiscontinuity {
    /// The epoch after the boundary.
    pub epoch: u64,
    /// The reason for the boundary.
    pub reason: PipelineDiscontinuityReason,
}

/// Stable pipeline-level names for presentation-clock boundaries.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PipelineDiscontinuityReason {
    /// A seek took effect.
    Seek,
    /// Output paused.
    Pause,
    /// Output resumed.
    Resume,
    /// The active source changed.
    SourceTransition,
    /// The output path restarted.
    OutputRestart,
    /// An underrun recovery boundary occurred.
    UnderrunRecovery,
    /// Playback rate changed.
    PlaybackRateChanged,
    /// Output sample rate changed.
    SampleRateChanged,
    /// A stop reset the media position.
    Stop,
}

impl From<DiscontinuityReason> for PipelineDiscontinuityReason {
    fn from(reason: DiscontinuityReason) -> Self {
        match reason {
            DiscontinuityReason::Seek => Self::Seek,
            DiscontinuityReason::Pause => Self::Pause,
            DiscontinuityReason::Resume => Self::Resume,
            DiscontinuityReason::SourceTransition => Self::SourceTransition,
            DiscontinuityReason::OutputRestart => Self::OutputRestart,
            DiscontinuityReason::UnderrunRecovery => Self::UnderrunRecovery,
            DiscontinuityReason::PlaybackRateChanged => Self::PlaybackRateChanged,
            DiscontinuityReason::SampleRateChanged => Self::SampleRateChanged,
            DiscontinuityReason::Stop => Self::Stop,
        }
    }
}

/// Validated bounds for one offline pipeline.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PipelineConfig {
    output_sample_rate_hz: u32,
    channels: usize,
    block_frames: usize,
    max_buffer_frames: u64,
    prebuffer_frames: u64,
}

impl PipelineConfig {
    /// Creates a bounded local-file configuration.
    ///
    /// The prebuffer target is expressed in output frames and must fit in the
    /// fixed buffer capacity.
    ///
    /// # Errors
    ///
    /// Returns a typed error when dimensions or bounds are invalid.
    pub fn new(
        output_sample_rate_hz: u32,
        channels: usize,
        block_frames: usize,
        max_buffer_frames: u64,
        prebuffer_frames: u64,
    ) -> Result<Self, PipelineError> {
        if output_sample_rate_hz == 0 {
            return Err(PipelineError::InvalidConfiguration {
                field: "output_sample_rate_hz",
            });
        }
        if !(1..=8).contains(&channels) {
            return Err(PipelineError::InvalidConfiguration { field: "channels" });
        }
        if block_frames == 0 {
            return Err(PipelineError::InvalidConfiguration {
                field: "block_frames",
            });
        }
        if max_buffer_frames == 0 || prebuffer_frames == 0 || prebuffer_frames > max_buffer_frames {
            return Err(PipelineError::InvalidConfiguration {
                field: "buffer_bounds",
            });
        }
        let scratch_samples = block_frames
            .checked_mul(channels)
            .ok_or(PipelineError::CapacityOverflow)?;
        let max_samples = usize::try_from(max_buffer_frames)
            .ok()
            .and_then(|frames| frames.checked_mul(channels))
            .ok_or(PipelineError::CapacityOverflow)?;
        if scratch_samples > max_samples {
            return Err(PipelineError::InvalidConfiguration {
                field: "block_frames",
            });
        }
        Ok(Self {
            output_sample_rate_hz,
            channels,
            block_frames,
            max_buffer_frames,
            prebuffer_frames,
        })
    }

    /// Creates a default bounded configuration for the supplied sample rate.
    ///
    /// # Errors
    ///
    /// Returns a typed error when derived bounds overflow or are invalid.
    pub fn local_default(output_sample_rate_hz: u32) -> Result<Self, PipelineError> {
        let prebuffer_frames = u64::from(output_sample_rate_hz)
            .checked_mul(u64::from(DEFAULT_PREBUFFER_MS))
            .ok_or(PipelineError::CapacityOverflow)?
            .div_ceil(1_000);
        let max_buffer_frames = prebuffer_frames
            .checked_mul(2)
            .ok_or(PipelineError::CapacityOverflow)?;
        Self::new(
            output_sample_rate_hz,
            DEFAULT_CHANNELS,
            DEFAULT_BLOCK_FRAMES,
            max_buffer_frames,
            prebuffer_frames,
        )
    }

    /// Returns the output sample rate.
    #[must_use]
    pub const fn output_sample_rate_hz(self) -> u32 {
        self.output_sample_rate_hz
    }

    /// Returns the interleaved channel count.
    #[must_use]
    pub const fn channels(self) -> usize {
        self.channels
    }

    /// Returns the decoder scratch size in frames.
    #[must_use]
    pub const fn block_frames(self) -> usize {
        self.block_frames
    }

    /// Returns the fixed maximum buffered frames.
    #[must_use]
    pub const fn max_buffer_frames(self) -> u64 {
        self.max_buffer_frames
    }

    /// Returns the prebuffer target in frames.
    #[must_use]
    pub const fn prebuffer_frames(self) -> u64 {
        self.prebuffer_frames
    }

    fn block_samples(self) -> usize {
        self.block_frames * self.channels
    }

    fn max_buffer_samples(self) -> usize {
        usize::try_from(self.max_buffer_frames).expect("validated buffer frames fit usize")
            * self.channels
    }
}

/// An output/result projection after one pipeline command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PipelineUpdate {
    /// The latest playback session projection.
    pub session: PlaybackSnapshot,
    /// The latest queue projection.
    pub queue: QueueSnapshot,
    /// The queue transition caused by this operation, if any.
    pub transition: Option<QueueTransition>,
    /// Whether the command was accepted by the pipeline.
    pub accepted: bool,
}

/// The result of one prepared-output render operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RenderPreparedResult {
    /// Number of output frames supplied by the caller.
    pub requested_frames: usize,
    /// Number of frames copied from the current generation.
    pub rendered_frames: usize,
    /// Number of silent frames caused by an empty/short buffer.
    pub silent_frames: usize,
    /// Current generation after rendering.
    pub buffer_generation: u64,
    /// Whether the decoder has reached EOF.
    pub end_of_stream: bool,
}

/// A worker-plane progress result.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PumpResult {
    /// Number of interleaved samples decoded in this call.
    pub decoded_samples: usize,
    /// Buffered frames after the worker call.
    pub buffered_frames: u64,
    /// Whether the decoder has reached EOF.
    pub end_of_stream: bool,
    /// Whether the prebuffer target is currently satisfied.
    pub prebuffer_ready: bool,
    /// Whether the decoder failed during this worker call.
    pub failed: bool,
}

/// Errors that preserve the pipeline boundary without exposing runtime data.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PipelineError {
    /// A config field violates a bounded invariant.
    InvalidConfiguration { field: &'static str },
    /// A checked capacity calculation overflowed.
    CapacityOverflow,
    /// The supplied output is not interleaved-frame aligned.
    MisalignedOutput { samples: usize, channels: usize },
    /// A decoder returned more samples than its scratch buffer.
    DecoderOutputTooLarge {
        samples_written: usize,
        capacity: usize,
    },
    /// A decoder made no progress without reaching EOF.
    DecoderNoProgress,
    /// A worker result belongs to a retired generation.
    StaleGeneration { actual: u64, expected: u64 },
    /// A source could not be resolved.
    SourceUnavailable,
    /// A decoder failed after source resolution.
    DecoderFailure,
    /// A bounded PCM buffer cannot accept more samples.
    BufferFull,
    /// The queue rejected a mutation.
    Queue(QueueError),
    /// The playback session rejected a checked state update.
    Session(SessionError),
    /// The output sink rejected a capture.
    SinkFailure,
    /// An internally reserved request identity was exhausted.
    RequestIdExhausted,
    /// A caller tried to play an identity that is not in this queue.
    ItemNotInQueue,
}

impl fmt::Display for PipelineError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidConfiguration { field } => {
                write!(formatter, "invalid offline pipeline configuration: {field}")
            }
            Self::CapacityOverflow => formatter.write_str("offline pipeline capacity overflowed"),
            Self::MisalignedOutput { samples, channels } => {
                write!(
                    formatter,
                    "output has {samples} samples for {channels} channels"
                )
            }
            Self::DecoderOutputTooLarge {
                samples_written,
                capacity,
            } => write!(
                formatter,
                "decoder wrote {samples_written} samples into capacity {capacity}"
            ),
            Self::DecoderNoProgress => formatter.write_str("decoder made no progress before EOF"),
            Self::StaleGeneration { actual, expected } => {
                write!(
                    formatter,
                    "buffer generation {actual} is stale; expected {expected}"
                )
            }
            Self::SourceUnavailable => formatter.write_str("offline source is unavailable"),
            Self::DecoderFailure => formatter.write_str("offline decoder failed"),
            Self::BufferFull => formatter.write_str("offline PCM buffer is full"),
            Self::Queue(error) => error.fmt(formatter),
            Self::Session(error) => error.fmt(formatter),
            Self::SinkFailure => formatter.write_str("offline output sink rejected capture"),
            Self::RequestIdExhausted => {
                formatter.write_str("internal request ID space is exhausted")
            }
            Self::ItemNotInQueue => formatter.write_str("requested item is not in the queue"),
        }
    }
}

impl Error for PipelineError {}

impl From<QueueError> for PipelineError {
    fn from(error: QueueError) -> Self {
        Self::Queue(error)
    }
}

impl From<SessionError> for PipelineError {
    fn from(error: SessionError) -> Self {
        Self::Session(error)
    }
}

#[derive(Debug, Clone)]
struct PcmBuffer {
    generation: u64,
    samples: VecDeque<f32>,
    capacity_samples: usize,
}

impl PcmBuffer {
    fn new(capacity_samples: usize) -> Self {
        Self {
            generation: 0,
            samples: VecDeque::with_capacity(capacity_samples),
            capacity_samples,
        }
    }

    fn generation(&self) -> u64 {
        self.generation
    }

    fn reset(&mut self, generation: u64) {
        self.samples.clear();
        self.generation = generation;
    }

    fn push(&mut self, generation: u64, samples: &[f32]) -> Result<(), PipelineError> {
        if generation != self.generation {
            return Err(PipelineError::StaleGeneration {
                actual: generation,
                expected: self.generation,
            });
        }
        let required = self
            .samples
            .len()
            .checked_add(samples.len())
            .ok_or(PipelineError::CapacityOverflow)?;
        if required > self.capacity_samples {
            return Err(PipelineError::BufferFull);
        }
        self.samples.extend(samples.iter().copied());
        Ok(())
    }

    fn drain(&mut self, output: &mut [f32]) -> usize {
        let count = output.len().min(self.samples.len());
        for destination in output.iter_mut().take(count) {
            *destination = self.samples.pop_front().unwrap_or(0.0);
        }
        count
    }

    fn buffered_frames(&self, channels: usize) -> u64 {
        u64::try_from(self.samples.len() / channels).unwrap_or(u64::MAX)
    }

    fn is_empty(&self) -> bool {
        self.samples.is_empty()
    }
}

/// A deterministic offline local playback pipeline.
pub struct OfflinePlaybackPipeline<R, S> {
    resolver: R,
    sink: S,
    config: PipelineConfig,
    session: PlaybackSession,
    queue: PlaybackQueue,
    decoder: Option<Box<dyn OfflineDecoder>>,
    buffer: PcmBuffer,
    scratch: Vec<f32>,
    decoder_eof: bool,
    internal_request_id: u64,
    last_error: Option<PipelineError>,
    active_worker_token: Option<OperationToken>,
    seen_request_ids: BTreeSet<RequestId>,
}

impl<R, S> OfflinePlaybackPipeline<R, S>
where
    R: OfflineSourceResolver,
    S: OfflineOutputSink,
{
    /// Creates an empty pipeline with preallocated decoder and buffer storage.
    ///
    /// # Errors
    ///
    /// Returns a typed error if the supplied configuration or output clock is
    /// invalid.
    pub fn new(resolver: R, sink: S, config: PipelineConfig) -> Result<Self, PipelineError> {
        let session = PlaybackSession::new(config.output_sample_rate_hz())?;
        Ok(Self {
            resolver,
            sink,
            config,
            session,
            queue: PlaybackQueue::new(),
            decoder: None,
            buffer: PcmBuffer::new(config.max_buffer_samples()),
            scratch: vec![0.0; config.block_samples()],
            decoder_eof: false,
            internal_request_id: u64::MAX,
            last_error: None,
            active_worker_token: None,
            seen_request_ids: BTreeSet::new(),
        })
    }

    /// Returns the latest session snapshot.
    #[must_use]
    pub fn session_snapshot(&self) -> PlaybackSnapshot {
        self.session.snapshot()
    }

    /// Returns the latest queue snapshot.
    #[must_use]
    pub fn queue_snapshot(&self) -> QueueSnapshot {
        self.queue.snapshot()
    }

    /// Returns the last worker failure, if one was classified.
    #[must_use]
    pub fn last_error(&self) -> Option<&PipelineError> {
        self.last_error.as_ref()
    }

    /// Returns the currently buffered frames.
    #[must_use]
    pub fn buffered_frames(&self) -> u64 {
        self.buffer.buffered_frames(self.config.channels())
    }

    /// Returns the active buffer generation.
    #[must_use]
    pub fn buffer_generation(&self) -> u64 {
        self.buffer.generation()
    }

    /// Returns an immutable view of the configured output sink.
    #[must_use]
    pub const fn sink(&self) -> &S {
        &self.sink
    }

    /// Returns a mutable view for reading a test/capture sink after a run.
    pub fn sink_mut(&mut self) -> &mut S {
        &mut self.sink
    }

    /// Dispatches a Core command through queue and session ownership.
    ///
    /// Queue mutations are handled here because Gate 2A intentionally leaves
    /// queue policy to this later batch. Source and decoder work triggered by a
    /// session intent are executed through the worker-plane adapter.
    ///
    /// # Errors
    ///
    /// Returns only bounded state, queue, or sink-independent pipeline errors.
    pub fn dispatch(&mut self, command: PlaybackCommand) -> Result<PipelineUpdate, PipelineError> {
        if !self.seen_request_ids.insert(command.request_id()) {
            return Ok(self.update(None, false));
        }
        match command.action() {
            PlaybackAction::SetQueue {
                items,
                current_index,
            } => {
                self.queue.replace(items.clone(), *current_index)?;
                Ok(self.update(None, true))
            }
            PlaybackAction::SetShuffle { enabled, seed } => {
                self.queue.set_shuffle(*enabled, *seed)?;
                Ok(self.update(None, true))
            }
            PlaybackAction::SetRepeat { mode } => {
                self.queue.set_repeat_mode(*mode);
                Ok(self.update(None, true))
            }
            PlaybackAction::Next => self.dispatch_next(command.request_id()),
            PlaybackAction::Previous => {
                let position = self.session.clock().media_position_us();
                self.dispatch_previous(command.request_id(), position)
            }
            PlaybackAction::Play { item_id } => {
                let selected = self.select_for_play(item_id.as_ref())?;
                let forwarded = PlaybackCommand::new(
                    command.request_id(),
                    PlaybackAction::Play {
                        item_id: Some(selected),
                    },
                );
                self.dispatch_session(forwarded, None)
            }
            _ => self.dispatch_session(command, None),
        }
    }

    /// Runs bounded worker decoding until the configured prebuffer target,
    /// buffer capacity, or decoder EOF is reached.
    ///
    /// This is a worker operation and must not be called from a realtime
    /// callback.
    ///
    /// # Errors
    ///
    /// Returns a typed bound violation. Decoder failures are classified into
    /// the session and returned as a progress result so the state converges.
    pub fn pump_worker(&mut self) -> Result<PumpResult, PipelineError> {
        if self.decoder.is_none() {
            return Ok(PumpResult {
                decoded_samples: 0,
                buffered_frames: self.buffered_frames(),
                end_of_stream: self.decoder_eof,
                prebuffer_ready: false,
                failed: false,
            });
        }
        let mut decoded_samples = 0;
        loop {
            let buffered_frames = self.buffered_frames();
            if buffered_frames >= self.config.prebuffer_frames() || self.decoder_eof {
                break;
            }
            let available_frames = self
                .config
                .max_buffer_frames()
                .saturating_sub(buffered_frames);
            let available_samples = usize::try_from(available_frames)
                .ok()
                .and_then(|frames| frames.checked_mul(self.config.channels()))
                .ok_or(PipelineError::CapacityOverflow)?
                .min(self.scratch.len());
            let output_samples = available_samples - (available_samples % self.config.channels());
            if output_samples == 0 {
                return Err(PipelineError::BufferFull);
            }
            let step = {
                let decoder = self.decoder.as_mut().ok_or(PipelineError::DecoderFailure)?;
                match decoder.decode(&mut self.scratch[..output_samples]) {
                    Ok(step) => step,
                    Err(error) => {
                        self.classify_decoder_failure(error);
                        return Ok(PumpResult {
                            decoded_samples,
                            buffered_frames: self.buffered_frames(),
                            end_of_stream: self.decoder_eof,
                            prebuffer_ready: false,
                            failed: true,
                        });
                    }
                }
            };
            if step.samples_written() > self.scratch.len() {
                self.classify_decoder_failure(PipelineError::DecoderOutputTooLarge {
                    samples_written: step.samples_written(),
                    capacity: self.scratch.len(),
                });
                return Ok(PumpResult {
                    decoded_samples,
                    buffered_frames: self.buffered_frames(),
                    end_of_stream: self.decoder_eof,
                    prebuffer_ready: false,
                    failed: true,
                });
            }
            let written = step.samples_written();
            if written % self.config.channels() != 0 {
                self.classify_decoder_failure(PipelineError::DecoderOutputTooLarge {
                    samples_written: written,
                    capacity: self.scratch.len(),
                });
                return Ok(PumpResult {
                    decoded_samples,
                    buffered_frames: self.buffered_frames(),
                    end_of_stream: self.decoder_eof,
                    prebuffer_ready: false,
                    failed: true,
                });
            }
            if written == 0 && !step.end_of_stream() {
                self.classify_decoder_failure(PipelineError::DecoderNoProgress);
                return Ok(PumpResult {
                    decoded_samples,
                    buffered_frames: self.buffered_frames(),
                    end_of_stream: self.decoder_eof,
                    prebuffer_ready: false,
                    failed: true,
                });
            }
            let generation = self.buffer.generation();
            self.buffer.push(generation, &self.scratch[..written])?;
            decoded_samples = decoded_samples
                .checked_add(written)
                .ok_or(PipelineError::CapacityOverflow)?;
            self.decoder_eof = step.end_of_stream();
        }
        Ok(PumpResult {
            decoded_samples,
            buffered_frames: self.buffered_frames(),
            end_of_stream: self.decoder_eof,
            prebuffer_ready: self.buffered_frames() >= self.config.prebuffer_frames()
                || self.decoder_eof,
            failed: false,
        })
    }

    /// Renders only prepared PCM into the output sink.
    ///
    /// No source, decoder, allocation, or wait occurs here. Call
    /// `pump_worker` separately to refill the bounded buffer.
    ///
    /// # Errors
    ///
    /// Returns alignment, clock, or sink errors.
    pub fn render_prepared(
        &mut self,
        output: &mut [f32],
    ) -> Result<RenderPreparedResult, PipelineError> {
        let channels = self.config.channels();
        if !output.len().is_multiple_of(channels) {
            return Err(PipelineError::MisalignedOutput {
                samples: output.len(),
                channels,
            });
        }
        output.fill(0.0);
        let requested_frames = output.len() / channels;
        let rendered_samples = if self.session.state() == SessionState::Playing {
            self.buffer.drain(output)
        } else {
            0
        };
        let rendered_frames = rendered_samples / channels;
        let silent_frames = requested_frames - rendered_frames;
        let after = self
            .session
            .record_rendered_frames(u64::try_from(rendered_frames).unwrap_or(u64::MAX))?;
        let clock = PipelineClockSample::from(after.clock());
        let discontinuity = if after.clock().is_discontinuous() {
            after
                .clock()
                .discontinuity_reason()
                .map(|reason| PipelineDiscontinuity {
                    epoch: after.clock().clock_epoch(),
                    reason: PipelineDiscontinuityReason::from(reason),
                })
        } else {
            None
        };
        self.sink.write(output, clock, discontinuity)?;
        if discontinuity.is_some() {
            self.session.acknowledge_discontinuity();
        }
        if self.decoder_eof
            && self.buffer.is_empty()
            && self.session.state() == SessionState::Playing
        {
            self.finish_current_item()?;
        }
        Ok(RenderPreparedResult {
            requested_frames,
            rendered_frames,
            silent_frames,
            buffer_generation: self.buffer.generation(),
            end_of_stream: self.decoder_eof,
        })
    }

    /// Returns the configured pipeline limits.
    #[must_use]
    pub const fn config(&self) -> PipelineConfig {
        self.config
    }

    fn dispatch_session(
        &mut self,
        command: PlaybackCommand,
        transition: Option<QueueTransition>,
    ) -> Result<PipelineUpdate, PipelineError> {
        let update = self.session.dispatch(command)?;
        let intent = update.intent().cloned();
        if let Some(intent) = intent {
            self.execute_intent(intent)?;
        }
        Ok(self.update(transition, update.result().is_accepted()))
    }

    fn dispatch_next(&mut self, request_id: RequestId) -> Result<PipelineUpdate, PipelineError> {
        let transition = self.queue.next();
        self.dispatch_transition(request_id, transition)
    }

    fn dispatch_previous(
        &mut self,
        request_id: RequestId,
        position_us: u64,
    ) -> Result<PipelineUpdate, PipelineError> {
        let transition = self.queue.previous(position_us);
        self.dispatch_transition(request_id, transition)
    }

    fn dispatch_transition(
        &mut self,
        request_id: RequestId,
        transition: QueueTransition,
    ) -> Result<PipelineUpdate, PipelineError> {
        if let Some(item_id) = transition.item_id().cloned() {
            self.stop_for_transition()?;
            let command = PlaybackCommand::new(
                request_id,
                PlaybackAction::Play {
                    item_id: Some(item_id),
                },
            );
            self.dispatch_session(command, Some(transition))
        } else {
            if transition == QueueTransition::Ended {
                self.end_current_for_queue()?;
            }
            Ok(self.update(Some(transition), true))
        }
    }

    fn select_for_play(
        &mut self,
        requested: Option<&PlaybackItemId>,
    ) -> Result<PlaybackItemId, PipelineError> {
        if let Some(item_id) = requested {
            let Some(index) = self.queue.items().iter().position(|item| item == item_id) else {
                return Err(PipelineError::ItemNotInQueue);
            };
            self.queue.select(index)?;
            return Ok(item_id.clone());
        }
        if self.queue.current_item().is_none() {
            let _ = self.queue.first();
        }
        self.queue
            .current_item()
            .cloned()
            .ok_or(PipelineError::ItemNotInQueue)
    }

    fn stop_for_transition(&mut self) -> Result<(), PipelineError> {
        if matches!(
            self.session.state(),
            SessionState::Empty
                | SessionState::Stopped
                | SessionState::Ended
                | SessionState::Failed
                | SessionState::Unavailable
        ) {
            self.decoder = None;
            self.buffer.reset(self.buffer.generation());
            self.decoder_eof = false;
            return Ok(());
        }
        let request_id = self.next_internal_request_id()?;
        let command = PlaybackCommand::new(request_id, PlaybackAction::Stop);
        let update = self.session.dispatch(command)?;
        if let Some(intent) = update.intent().cloned() {
            self.execute_intent(intent)?;
        }
        Ok(())
    }

    fn execute_intent(&mut self, intent: WorkerIntent) -> Result<(), PipelineError> {
        match intent {
            WorkerIntent::ResolveSource { token, item_id } => {
                self.buffer.reset(token.buffer_generation());
                self.decoder_eof = false;
                self.last_error = None;
                match self.resolver.open(&item_id) {
                    Ok(decoder) => {
                        self.decoder = Some(decoder);
                        self.apply_worker_event(WorkerEvent::SourceReady { token })
                    }
                    Err(PipelineError::SourceUnavailable) => {
                        self.decoder = None;
                        self.apply_worker_event(WorkerEvent::SourceUnavailable { token })
                    }
                    Err(error) => {
                        self.last_error = Some(error);
                        self.decoder = None;
                        self.apply_worker_event(WorkerEvent::Failed { token })
                    }
                }
            }
            WorkerIntent::PrepareBuffer { token, .. } => {
                self.buffer.reset(token.buffer_generation());
                let progress = self.pump_worker()?;
                if progress.failed {
                    self.apply_worker_event(WorkerEvent::Failed { token })
                } else if progress.prebuffer_ready {
                    self.apply_worker_event(WorkerEvent::PrebufferReady { token })
                } else {
                    Ok(())
                }
            }
            WorkerIntent::Seek { token, position_us } => {
                self.buffer.reset(token.buffer_generation());
                self.decoder_eof = false;
                let result = self
                    .decoder
                    .as_mut()
                    .ok_or(PipelineError::DecoderFailure)
                    .and_then(|decoder| decoder.seek(position_us));
                match result {
                    Ok(step) => self.apply_worker_event(WorkerEvent::SeekApplied {
                        token,
                        position_us: step.effective_position_us(),
                    }),
                    Err(error) => {
                        self.classify_decoder_failure(error);
                        self.apply_worker_event(WorkerEvent::Failed { token })
                    }
                }
            }
            WorkerIntent::Pause { token } => {
                self.buffer.reset(token.buffer_generation());
                Ok(())
            }
            WorkerIntent::Stop { token } => {
                self.decoder = None;
                self.buffer.reset(token.buffer_generation());
                self.decoder_eof = false;
                self.active_worker_token = None;
                Ok(())
            }
        }
    }

    fn apply_worker_event(&mut self, event: WorkerEvent) -> Result<(), PipelineError> {
        let token = event.token();
        self.active_worker_token = Some(token);
        let update = self.session.handle_worker_event(event)?;
        if matches!(
            update.snapshot().state(),
            SessionState::Ended | SessionState::Failed | SessionState::Unavailable
        ) {
            self.active_worker_token = None;
        }
        if let Some(intent) = update.intent().cloned() {
            self.execute_intent(intent)?;
        }
        Ok(())
    }

    fn classify_decoder_failure(&mut self, error: PipelineError) {
        self.last_error = Some(error);
        self.decoder = None;
    }

    fn finish_current_item(&mut self) -> Result<(), PipelineError> {
        let Some(token) = self.active_worker_token else {
            return Ok(());
        };
        self.apply_worker_event(WorkerEvent::Ended { token })?;
        let transition = self.queue.complete_current();
        if let Some(item_id) = transition.item_id().cloned() {
            self.decoder = None;
            self.buffer.reset(self.buffer.generation());
            self.decoder_eof = false;
            let request_id = self.next_internal_request_id()?;
            let command = PlaybackCommand::new(
                request_id,
                PlaybackAction::Play {
                    item_id: Some(item_id),
                },
            );
            let update = self.session.dispatch(command)?;
            if let Some(intent) = update.intent().cloned() {
                self.execute_intent(intent)?;
            }
        } else {
            self.decoder = None;
            self.decoder_eof = false;
        }
        Ok(())
    }

    fn end_current_for_queue(&mut self) -> Result<(), PipelineError> {
        if let Some(token) = self.active_worker_token {
            self.apply_worker_event(WorkerEvent::Ended { token })?;
        }
        self.decoder = None;
        self.buffer.reset(self.buffer.generation());
        self.decoder_eof = false;
        Ok(())
    }

    fn next_internal_request_id(&mut self) -> Result<RequestId, PipelineError> {
        loop {
            let request_id = RequestId::new(self.internal_request_id);
            self.internal_request_id = self
                .internal_request_id
                .checked_sub(1)
                .ok_or(PipelineError::RequestIdExhausted)?;
            if self.seen_request_ids.insert(request_id) {
                return Ok(request_id);
            }
        }
    }

    fn update(&self, transition: Option<QueueTransition>, accepted: bool) -> PipelineUpdate {
        PipelineUpdate {
            session: self.session.snapshot(),
            queue: self.queue.snapshot(),
            transition,
            accepted,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        DecodeStep, OfflineDecoder, OfflineOutputSink, OfflinePlaybackPipeline,
        OfflineSourceResolver, PipelineClockSample, PipelineConfig, PipelineDiscontinuity,
        PipelineError, SeekStep,
    };
    use crate::command::{PlaybackAction, PlaybackCommand, PlaybackItemId, RequestId};

    #[derive(Debug)]
    struct FixtureDecoder {
        samples: Vec<f32>,
        cursor: usize,
    }

    impl OfflineDecoder for FixtureDecoder {
        fn decode(&mut self, output: &mut [f32]) -> Result<DecodeStep, PipelineError> {
            let remaining = self.samples.len().saturating_sub(self.cursor);
            let count = remaining.min(output.len());
            output[..count].copy_from_slice(&self.samples[self.cursor..self.cursor + count]);
            self.cursor += count;
            Ok(DecodeStep::new(count, self.cursor == self.samples.len()))
        }

        fn seek(&mut self, position_us: u64) -> Result<SeekStep, PipelineError> {
            let frame = usize::try_from(position_us / 1_000).unwrap_or(usize::MAX);
            self.cursor = frame
                .checked_mul(2)
                .unwrap_or(self.samples.len())
                .min(self.samples.len());
            Ok(SeekStep::new(position_us))
        }
    }

    #[derive(Debug)]
    struct FixtureResolver {
        samples: Vec<f32>,
    }

    impl OfflineSourceResolver for FixtureResolver {
        fn open(
            &mut self,
            _item_id: &PlaybackItemId,
        ) -> Result<Box<dyn OfflineDecoder>, PipelineError> {
            Ok(Box::new(FixtureDecoder {
                samples: self.samples.clone(),
                cursor: 0,
            }))
        }
    }

    #[derive(Debug, Default)]
    struct FixtureSink {
        samples: Vec<f32>,
        clocks: Vec<PipelineClockSample>,
        discontinuities: Vec<PipelineDiscontinuity>,
    }

    impl OfflineOutputSink for FixtureSink {
        fn write(
            &mut self,
            samples: &[f32],
            clock: PipelineClockSample,
            discontinuity: Option<PipelineDiscontinuity>,
        ) -> Result<(), PipelineError> {
            self.samples.extend_from_slice(samples);
            self.clocks.push(clock);
            if let Some(discontinuity) = discontinuity {
                self.discontinuities.push(discontinuity);
            }
            Ok(())
        }
    }

    fn item(value: &str) -> PlaybackItemId {
        PlaybackItemId::new(value).expect("fixture identity is valid")
    }

    fn pipeline(samples: Vec<f32>) -> OfflinePlaybackPipeline<FixtureResolver, FixtureSink> {
        let config = PipelineConfig::new(1_000, 2, 2, 8, 2).expect("fixture config is valid");
        OfflinePlaybackPipeline::new(FixtureResolver { samples }, FixtureSink::default(), config)
            .expect("fixture pipeline is valid")
    }

    #[test]
    fn command_to_prepared_output_is_deterministic() {
        let mut first = pipeline(vec![0.1, 0.2, 0.3, 0.4]);
        let mut second = pipeline(vec![0.1, 0.2, 0.3, 0.4]);
        let set_queue = PlaybackCommand::new(
            RequestId::new(1),
            PlaybackAction::SetQueue {
                items: vec![item("track-a")],
                current_index: Some(0),
            },
        );
        first.dispatch(set_queue.clone()).expect("queue accepted");
        second.dispatch(set_queue).expect("queue accepted");
        let play = PlaybackCommand::new(RequestId::new(2), PlaybackAction::Play { item_id: None });
        first.dispatch(play.clone()).expect("play accepted");
        second.dispatch(play).expect("play accepted");
        let mut first_output = [0.0; 4];
        let mut second_output = [0.0; 4];
        first
            .render_prepared(&mut first_output)
            .expect("first render succeeds");
        second
            .render_prepared(&mut second_output)
            .expect("second render succeeds");
        assert!(
            first_output
                .iter()
                .zip(second_output)
                .all(|(actual, expected)| actual.to_bits() == expected.to_bits())
        );
        assert_eq!(first.buffered_frames(), second.buffered_frames());
    }

    #[test]
    fn seek_retires_old_pcm_and_emits_new_epoch() {
        let mut pipeline = pipeline(vec![1.0, 1.0, 2.0, 2.0]);
        pipeline
            .dispatch(PlaybackCommand::new(
                RequestId::new(1),
                PlaybackAction::SetQueue {
                    items: vec![item("track-a")],
                    current_index: Some(0),
                },
            ))
            .expect("queue accepted");
        pipeline
            .dispatch(PlaybackCommand::new(
                RequestId::new(2),
                PlaybackAction::Play { item_id: None },
            ))
            .expect("play accepted");
        pipeline
            .dispatch(PlaybackCommand::new(
                RequestId::new(3),
                PlaybackAction::Seek { position_us: 1_000 },
            ))
            .expect("seek accepted");
        let mut output = [0.0; 2];
        pipeline
            .render_prepared(&mut output)
            .expect("render succeeds");
        assert!(
            output
                .iter()
                .all(|sample| sample.to_bits() == 2.0_f32.to_bits())
        );
    }

    #[test]
    fn pause_does_not_consume_prepared_pcm_or_advance_clock() {
        let mut pipeline = pipeline(vec![0.1, 0.2, 0.3, 0.4]);
        pipeline
            .dispatch(PlaybackCommand::new(
                RequestId::new(1),
                PlaybackAction::SetQueue {
                    items: vec![item("track-a")],
                    current_index: Some(0),
                },
            ))
            .expect("queue accepted");
        pipeline
            .dispatch(PlaybackCommand::new(
                RequestId::new(2),
                PlaybackAction::Play { item_id: None },
            ))
            .expect("play accepted");
        let before = pipeline.session_snapshot().clock();
        pipeline
            .dispatch(PlaybackCommand::new(
                RequestId::new(3),
                PlaybackAction::Pause,
            ))
            .expect("pause accepted");
        let mut output = [0.0; 2];
        pipeline
            .render_prepared(&mut output)
            .expect("paused render succeeds");
        assert_eq!(
            pipeline.session_snapshot().clock().rendered_frames(),
            before.rendered_frames()
        );
    }
}
