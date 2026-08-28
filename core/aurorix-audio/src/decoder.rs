//! Format-neutral decoder and worker lifecycle abstractions.
//!
//! Concrete WAV/FLAC/MP3/AAC/Opus implementations belong to later Gate 2
//! batches. This module only defines the worker contract that keeps source I/O
//! and decoder work off the realtime callback path.

use crate::{
    errors::{DecoderError, SourceError},
    seek::{
        BufferGeneration, DecoderDelayPadding, GenerationCause, SeekCoordinator, SeekMetadata,
        SeekResult, SeekTarget,
    },
    source::RuntimeSource,
};

/// Default upper bound for one worker decode output call, measured in samples.
pub const DEFAULT_MAX_OUTPUT_SAMPLES: usize = 8 * 1024;

/// Lifecycle state of a decoder worker.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkerState {
    /// A worker exists but has not started consuming source data.
    Ready,
    /// The worker may call the decoder and produce output.
    Running,
    /// The worker retains its decoder position without consuming output.
    Paused,
    /// The decoder reported end of stream.
    Ended,
    /// A non-cancellation decoder failure retired the worker.
    Failed,
    /// Cancellation retired the source and decoder.
    Cancelled,
    /// The worker has released its runtime resources.
    Closed,
}

impl WorkerState {
    const fn label(self) -> &'static str {
        match self {
            Self::Ready => "ready",
            Self::Running => "running",
            Self::Paused => "paused",
            Self::Ended => "ended",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
            Self::Closed => "closed",
        }
    }
}

/// Decoder output written into the caller-provided PCM scratch buffer.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DecodeOutput {
    samples_written: usize,
    end_of_stream: bool,
    delay_padding: DecoderDelayPadding,
}

impl DecodeOutput {
    /// Creates a decoder result without delay/padding metadata.
    #[must_use]
    pub const fn new(samples_written: usize, end_of_stream: bool) -> Self {
        Self {
            samples_written,
            end_of_stream,
            delay_padding: DecoderDelayPadding::new(0, 0),
        }
    }

    /// Creates a decoder result with codec delay/padding metadata.
    #[must_use]
    pub const fn with_delay_padding(
        samples_written: usize,
        end_of_stream: bool,
        delay_padding: DecoderDelayPadding,
    ) -> Self {
        Self {
            samples_written,
            end_of_stream,
            delay_padding,
        }
    }

    /// Returns the number of valid samples written to the supplied buffer.
    #[must_use]
    pub const fn samples_written(self) -> usize {
        self.samples_written
    }

    /// Returns whether this output reaches decoder EOF.
    #[must_use]
    pub const fn end_of_stream(self) -> bool {
        self.end_of_stream
    }

    /// Returns delay/padding metadata associated with this output.
    #[must_use]
    pub const fn delay_padding(self) -> DecoderDelayPadding {
        self.delay_padding
    }
}

/// A worker output descriptor tagged with the generation that produced it.
/// The samples themselves remain in the caller-owned output slice.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WorkerOutput {
    generation: BufferGeneration,
    samples_written: usize,
    end_of_stream: bool,
    delay_padding: DecoderDelayPadding,
}

impl WorkerOutput {
    /// Returns the buffer generation associated with the output.
    #[must_use]
    pub const fn generation(self) -> BufferGeneration {
        self.generation
    }

    /// Returns the number of valid samples in the caller's output buffer.
    #[must_use]
    pub const fn samples_written(self) -> usize {
        self.samples_written
    }

    /// Returns whether the output reaches EOF.
    #[must_use]
    pub const fn end_of_stream(self) -> bool {
        self.end_of_stream
    }

    /// Returns delay/padding metadata associated with the output.
    #[must_use]
    pub const fn delay_padding(self) -> DecoderDelayPadding {
        self.delay_padding
    }
}

/// A format-neutral decoder implementation supplied by a later codec module.
pub trait Decoder: Send {
    /// Decodes into a caller-owned, bounded PCM scratch buffer.
    ///
    /// # Errors
    ///
    /// Returns a typed source, unsupported-format, corrupt-input, cancellation,
    /// or decoder failure. Implementations must not write beyond `output`.
    fn decode(
        &mut self,
        source: &mut dyn RuntimeSource,
        output: &mut [f32],
    ) -> Result<DecodeOutput, DecoderError>;

    /// Seeks decoder state after the worker has serialized the operation.
    ///
    /// # Errors
    ///
    /// Returns a typed source, cancellation, unsupported-target, or decoder
    /// failure without exposing runtime handles or persistent locator state.
    fn seek(
        &mut self,
        source: &mut dyn RuntimeSource,
        target: SeekTarget,
    ) -> Result<SeekMetadata, DecoderError>;

    /// Rebuilds codec state after the source has been reopened.
    ///
    /// # Errors
    ///
    /// Returns a typed decoder failure when the reopened source cannot be
    /// accepted by the decoder.
    fn reopen(&mut self) -> Result<(), DecoderError> {
        Ok(())
    }

    /// Releases decoder-owned state. The worker still closes the source.
    ///
    /// # Errors
    ///
    /// Returns a typed decoder failure when the implementation cannot complete
    /// its explicit close operation.
    fn close(&mut self) -> Result<(), DecoderError>;

    /// Requests cooperative cancellation without touching realtime state.
    fn cancel(&mut self);
}

/// Bounded worker configuration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DecoderWorkerConfig {
    max_output_samples: usize,
}

impl DecoderWorkerConfig {
    /// Creates a worker configuration with a positive output bound.
    ///
    /// # Errors
    ///
    /// Returns [`DecoderError::InvalidConfiguration`] when the bound is zero.
    pub fn new(max_output_samples: usize) -> Result<Self, DecoderError> {
        if max_output_samples == 0 {
            return Err(DecoderError::InvalidConfiguration {
                field: "max_output_samples",
                value: 0,
            });
        }
        Ok(Self { max_output_samples })
    }

    /// Returns the maximum samples accepted by one decode call.
    #[must_use]
    pub const fn max_output_samples(self) -> usize {
        self.max_output_samples
    }
}

impl Default for DecoderWorkerConfig {
    fn default() -> Self {
        Self {
            max_output_samples: DEFAULT_MAX_OUTPUT_SAMPLES,
        }
    }
}

/// A worker that owns one decoder and one runtime source.
pub struct DecoderWorker<D> {
    decoder: D,
    source: Box<dyn RuntimeSource>,
    state: WorkerState,
    config: DecoderWorkerConfig,
    seeks: SeekCoordinator,
}

impl<D: Decoder> DecoderWorker<D> {
    /// Creates a ready worker using the default bounded output size.
    #[must_use]
    pub fn new(decoder: D, source: Box<dyn RuntimeSource>) -> Self {
        Self {
            decoder,
            source,
            state: WorkerState::Ready,
            config: DecoderWorkerConfig::default(),
            seeks: SeekCoordinator::new(),
        }
    }

    /// Creates a ready worker with explicit bounded output configuration.
    pub fn with_config(
        decoder: D,
        source: Box<dyn RuntimeSource>,
        config: DecoderWorkerConfig,
    ) -> Self {
        Self {
            decoder,
            source,
            state: WorkerState::Ready,
            config,
            seeks: SeekCoordinator::new(),
        }
    }

    /// Returns the current lifecycle state.
    #[must_use]
    pub const fn state(&self) -> WorkerState {
        self.state
    }

    /// Returns the current accepted buffer generation.
    #[must_use]
    pub const fn generation(&self) -> BufferGeneration {
        self.seeks.generation()
    }

    /// Returns the active worker bounds.
    #[must_use]
    pub const fn config(&self) -> DecoderWorkerConfig {
        self.config
    }

    /// Starts or resumes decoding.
    ///
    /// # Errors
    ///
    /// Returns [`DecoderError::InvalidState`] unless the worker is ready or
    /// paused.
    pub fn start(&mut self) -> Result<(), DecoderError> {
        match self.state {
            WorkerState::Ready | WorkerState::Paused => {
                self.state = WorkerState::Running;
                Ok(())
            }
            state => Err(invalid_state("start", state)),
        }
    }

    /// Pauses decoding while retaining the decoder position.
    ///
    /// # Errors
    ///
    /// Returns [`DecoderError::InvalidState`] unless the worker is running.
    pub fn pause(&mut self) -> Result<(), DecoderError> {
        if self.state != WorkerState::Running {
            return Err(invalid_state("pause", self.state));
        }
        self.state = WorkerState::Paused;
        Ok(())
    }

    /// Resumes a paused worker.
    ///
    /// # Errors
    ///
    /// Returns [`DecoderError::InvalidState`] unless the worker is paused.
    pub fn resume(&mut self) -> Result<(), DecoderError> {
        if self.state != WorkerState::Paused {
            return Err(invalid_state("resume", self.state));
        }
        self.state = WorkerState::Running;
        Ok(())
    }

    /// Decodes one bounded worker step into `output`.
    ///
    /// # Errors
    ///
    /// Returns [`DecoderError::InvalidState`] outside the running state,
    /// [`DecoderError::InvalidOutput`] for an over-bound buffer or invalid
    /// decoder count, and classified source/decoder failures otherwise.
    pub fn decode_step(&mut self, output: &mut [f32]) -> Result<WorkerOutput, DecoderError> {
        if self.state != WorkerState::Running {
            return Err(invalid_state("decode", self.state));
        }
        if output.len() > self.config.max_output_samples() {
            return Err(DecoderError::InvalidOutput {
                samples_written: output.len(),
                capacity: self.config.max_output_samples(),
            });
        }
        if self.source.is_cancelled() {
            self.state = WorkerState::Cancelled;
            return Err(DecoderError::Cancelled);
        }

        let generation = self.generation();
        let decoded = match self.decoder.decode(self.source.as_mut(), output) {
            Ok(decoded) => decoded,
            Err(error) => return Err(self.classify_failure(error)),
        };
        if decoded.samples_written() > output.len() {
            self.state = WorkerState::Failed;
            return Err(DecoderError::InvalidOutput {
                samples_written: decoded.samples_written(),
                capacity: output.len(),
            });
        }
        if decoded.end_of_stream() {
            self.state = WorkerState::Ended;
        }
        Ok(WorkerOutput {
            generation,
            samples_written: decoded.samples_written(),
            end_of_stream: decoded.end_of_stream(),
            delay_padding: decoded.delay_padding(),
        })
    }

    /// Performs a worker-side decoder seek without reopening the source.
    ///
    /// # Errors
    ///
    /// Returns a classified seek, source, cancellation, or decoder failure.
    pub fn seek(
        &mut self,
        request_id: u64,
        target: SeekTarget,
    ) -> Result<SeekResult, DecoderError> {
        self.seek_inner(request_id, target, false)
    }

    /// Reopens the runtime source, rebuilds decoder state, and seeks.
    ///
    /// # Errors
    ///
    /// Returns a classified reopen, seek, source, cancellation, or decoder
    /// failure. A failure cannot commit the new generation.
    pub fn reopen_and_seek(
        &mut self,
        request_id: u64,
        target: SeekTarget,
    ) -> Result<SeekResult, DecoderError> {
        self.seek_inner(request_id, target, true)
    }

    /// Invalidates the current output generation for an external worker event.
    ///
    /// # Errors
    ///
    /// Returns [`DecoderError::Seek`] when the generation counter cannot
    /// advance.
    pub fn invalidate_generation(
        &mut self,
        cause: GenerationCause,
    ) -> Result<BufferGeneration, DecoderError> {
        self.seeks.invalidate(cause).map_err(DecoderError::from)
    }

    /// Returns whether a worker output descriptor is still current.
    #[must_use]
    pub fn accepts_output(&self, output: WorkerOutput) -> bool {
        self.seeks.accepts(output.generation())
    }

    /// Cancels the worker and retires its source and output generation.
    ///
    /// # Errors
    ///
    /// Returns [`DecoderError::Seek`] only when the generation counter cannot
    /// advance after runtime resources are retired.
    pub fn cancel(&mut self) -> Result<(), DecoderError> {
        if self.state == WorkerState::Closed {
            return Ok(());
        }
        self.decoder.cancel();
        self.source.cancel();
        self.state = WorkerState::Cancelled;
        self.seeks
            .invalidate(GenerationCause::Cancellation)
            .map(|_| ())
            .map_err(DecoderError::from)
    }

    /// Closes decoder and source resources; closure is idempotent.
    ///
    /// # Errors
    ///
    /// Returns the decoder close failure first; otherwise returns a classified
    /// source close failure. The worker enters the closed state either way.
    pub fn close(&mut self) -> Result<(), DecoderError> {
        if self.state == WorkerState::Closed {
            return Ok(());
        }
        let decoder_result = self.decoder.close();
        let source_result = self.source.close().map_err(DecoderError::from);
        self.state = WorkerState::Closed;
        match decoder_result {
            Err(error) => Err(error),
            Ok(()) => source_result,
        }
    }

    fn seek_inner(
        &mut self,
        request_id: u64,
        target: SeekTarget,
        reopen: bool,
    ) -> Result<SeekResult, DecoderError> {
        match self.state {
            WorkerState::Ready
            | WorkerState::Running
            | WorkerState::Paused
            | WorkerState::Ended => {}
            state => return Err(invalid_state("seek", state)),
        }

        let was_running = self.state == WorkerState::Running;
        let was_paused = self.state == WorkerState::Paused;
        let plan = if reopen {
            self.seeks.begin_reopen(request_id, target)
        } else {
            self.seeks.begin_seek(request_id, target)
        }
        .map_err(DecoderError::from)?;

        if self.source.is_cancelled() {
            self.state = WorkerState::Cancelled;
            let _ = self.seeks.cancel(plan);
            return Err(DecoderError::Cancelled);
        }

        if reopen {
            if let Err(error) = self.source.reopen() {
                let _ = self.seeks.cancel(plan);
                return Err(self.classify_failure(error.into()));
            }
            if let Err(error) = self.decoder.reopen() {
                let _ = self.seeks.cancel(plan);
                return Err(self.classify_failure(error));
            }
        }

        let metadata = match self.decoder.seek(self.source.as_mut(), target) {
            Ok(metadata) => metadata,
            Err(error) => {
                let _ = self.seeks.cancel(plan);
                return Err(self.classify_failure(error));
            }
        };
        let result = self
            .seeks
            .commit(plan, metadata)
            .map_err(DecoderError::from)?;
        self.state = if was_running {
            WorkerState::Running
        } else if was_paused {
            WorkerState::Paused
        } else {
            WorkerState::Ready
        };
        Ok(result)
    }

    fn classify_failure(&mut self, error: DecoderError) -> DecoderError {
        match error {
            DecoderError::Cancelled | DecoderError::Source(SourceError::Cancelled) => {
                self.state = WorkerState::Cancelled;
                DecoderError::Cancelled
            }
            other => {
                self.state = WorkerState::Failed;
                other
            }
        }
    }
}

fn invalid_state(operation: &'static str, state: WorkerState) -> DecoderError {
    DecoderError::InvalidState {
        operation,
        state: state.label(),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        DEFAULT_MAX_OUTPUT_SAMPLES, DecodeOutput, Decoder, DecoderWorker, DecoderWorkerConfig,
        WorkerState,
    };
    use crate::{
        errors::{DecoderError, SourceError},
        seek::{DecoderDelayPadding, SeekMetadata, SeekTarget},
        source::{LocalFileSource, RuntimeSource},
    };
    use std::{
        fs,
        io::{SeekFrom, Write},
        path::PathBuf,
        time::{SystemTime, UNIX_EPOCH},
    };

    struct FixtureDecoder {
        closed: bool,
        cancelled: bool,
        fail_decode: bool,
    }

    impl FixtureDecoder {
        fn new() -> Self {
            Self {
                closed: false,
                cancelled: false,
                fail_decode: false,
            }
        }
    }

    impl Decoder for FixtureDecoder {
        fn decode(
            &mut self,
            source: &mut dyn RuntimeSource,
            output: &mut [f32],
        ) -> Result<DecodeOutput, DecoderError> {
            if self.fail_decode {
                return Err(DecoderError::corrupt("fixture decoder rejected input"));
            }
            let mut bytes = vec![0_u8; output.len().min(4)];
            let read = source.read_bounded(&mut bytes)?;
            for (sample, byte) in output.iter_mut().zip(bytes.iter()) {
                *sample = f32::from(*byte);
            }
            Ok(DecodeOutput::with_delay_padding(
                read.bytes_read(),
                read.end_of_stream(),
                DecoderDelayPadding::new(2, 3),
            ))
        }

        fn seek(
            &mut self,
            source: &mut dyn RuntimeSource,
            target: SeekTarget,
        ) -> Result<SeekMetadata, DecoderError> {
            match target {
                SeekTarget::ByteOffset(offset) => {
                    let position = source.seek(SeekFrom::Start(offset))?;
                    Ok(SeekMetadata::new(
                        DecoderDelayPadding::new(2, 3),
                        Some(position),
                    ))
                }
                _ => Err(DecoderError::Unsupported {
                    format: "fixture target".to_owned(),
                }),
            }
        }

        fn reopen(&mut self) -> Result<(), DecoderError> {
            self.closed = false;
            Ok(())
        }

        fn close(&mut self) -> Result<(), DecoderError> {
            self.closed = true;
            Ok(())
        }

        fn cancel(&mut self) {
            self.cancelled = true;
        }
    }

    fn fixture_path(label: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock must be after epoch")
            .as_nanos();
        std::env::temp_dir().join(format!("aurorix-decoder-{label}-{nanos}.bin"))
    }

    fn worker(label: &str) -> (DecoderWorker<FixtureDecoder>, PathBuf) {
        let path = fixture_path(label);
        let mut file = fs::File::create(&path).expect("fixture should be creatable");
        file.write_all(b"abcd").expect("fixture should be writable");
        let source = LocalFileSource::open(&path).expect("source should open");
        (
            DecoderWorker::new(FixtureDecoder::new(), Box::new(source)),
            path,
        )
    }

    #[test]
    fn worker_lifecycle_preserves_bounded_output_and_eof() {
        assert_eq!(DEFAULT_MAX_OUTPUT_SAMPLES, 8 * 1024);
        let (mut worker, path) = worker("lifecycle");
        assert_eq!(worker.state(), WorkerState::Ready);
        worker.start().expect("worker starts");
        let mut output = [0_f32; 4];
        let step = worker.decode_step(&mut output).expect("decode step");
        assert_eq!(step.samples_written(), 4);
        assert!(step.end_of_stream());
        assert_eq!(worker.state(), WorkerState::Ended);
        assert!(worker.accepts_output(step));
        worker.close().expect("worker closes");
        assert_eq!(worker.state(), WorkerState::Closed);
        assert!(worker.decoder.closed);
        fs::remove_file(path).expect("fixture cleanup");
    }

    #[test]
    fn seek_reopen_increments_generation_and_preserves_pause_state() {
        let (mut worker, path) = worker("seek");
        worker.start().expect("worker starts");
        worker.pause().expect("worker pauses");
        let result = worker
            .reopen_and_seek(9, SeekTarget::ByteOffset(2))
            .expect("seek should commit");
        assert_eq!(result.generation().value(), 1);
        assert_eq!(result.metadata().source_position(), Some(2));
        assert_eq!(worker.state(), WorkerState::Paused);
        assert_eq!(worker.generation().value(), 1);
        fs::remove_file(path).expect("fixture cleanup");
    }

    #[test]
    fn cancellation_closes_source_and_rejects_future_decode() {
        let (mut worker, path) = worker("cancel");
        worker.start().expect("worker starts");
        worker.cancel().expect("cancel should retire worker");
        assert_eq!(worker.state(), WorkerState::Cancelled);
        assert!(worker.decoder.cancelled);
        let mut output = [0_f32; 1];
        assert_eq!(
            worker.decode_step(&mut output),
            Err(DecoderError::InvalidState {
                operation: "decode",
                state: "cancelled",
            })
        );
        fs::remove_file(path).expect("fixture cleanup");
    }

    #[test]
    fn stale_output_is_rejected_after_generation_invalidation() {
        let (mut worker, path) = worker("stale");
        worker.start().expect("worker starts");
        let mut output = [0_f32; 2];
        let old = worker.decode_step(&mut output).expect("decode step");
        worker
            .invalidate_generation(crate::seek::GenerationCause::UnderrunRecovery)
            .expect("generation invalidates");
        assert!(!worker.accepts_output(old));
        fs::remove_file(path).expect("fixture cleanup");
    }

    #[test]
    fn decoder_failures_are_typed_and_retire_worker() {
        let (mut worker, path) = worker("failure");
        worker.decoder.fail_decode = true;
        worker.start().expect("worker starts");
        let mut output = [0_f32; 2];
        let error = worker
            .decode_step(&mut output)
            .expect_err("failure expected");
        assert!(matches!(error, DecoderError::Corrupt { .. }));
        assert_eq!(worker.state(), WorkerState::Failed);
        fs::remove_file(path).expect("fixture cleanup");
    }

    #[test]
    fn output_bound_is_rejected_before_decoder_invocation() {
        let path = fixture_path("bound");
        let mut file = fs::File::create(&path).expect("fixture should be creatable");
        file.write_all(b"abcd").expect("fixture should be writable");
        let config = DecoderWorkerConfig::new(1).expect("valid config");
        let mut worker = DecoderWorker::with_config(
            FixtureDecoder::new(),
            Box::new(LocalFileSource::open(&path).expect("source opens")),
            config,
        );
        worker.start().expect("worker starts");
        let mut output = [0_f32; 2];
        assert_eq!(
            worker.decode_step(&mut output),
            Err(DecoderError::InvalidOutput {
                samples_written: 2,
                capacity: 1,
            })
        );
        fs::remove_file(path).expect("fixture cleanup");
    }

    #[test]
    fn source_errors_do_not_leak_paths() {
        let error = DecoderError::from(SourceError::Missing);
        assert_eq!(error.to_string(), "decoder source is missing");
        assert!(!error.to_string().contains("Users"));
    }
}
