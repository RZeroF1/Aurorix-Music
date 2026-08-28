//! Worker-side seek coordination and stale-buffer protection.

use crate::errors::SeekError;

/// A monotonically increasing identity for one active decoded-buffer lifetime.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct BufferGeneration(u64);

impl BufferGeneration {
    /// The generation used before the first invalidating operation.
    pub const INITIAL: Self = Self(0);

    /// Creates a generation from a known counter value.
    #[must_use]
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    /// Returns the raw generation counter.
    #[must_use]
    pub const fn value(self) -> u64 {
        self.0
    }

    fn next(self) -> Result<Self, SeekError> {
        self.0
            .checked_add(1)
            .map(Self)
            .ok_or(SeekError::GenerationExhausted)
    }
}

impl Default for BufferGeneration {
    fn default() -> Self {
        Self::INITIAL
    }
}

/// The cause recorded for a generation invalidation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GenerationCause {
    /// A worker-side seek retired old decoded frames.
    Seek,
    /// A source or decoder was reopened.
    Reopen,
    /// The current source item changed.
    SourceTransition,
    /// An underrun recovery retired the old data path.
    UnderrunRecovery,
    /// Cancellation retired the worker's data path.
    Cancellation,
}

/// Tracks the only buffer generation accepted by a consumer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BufferGenerationTracker {
    current: BufferGeneration,
    last_cause: Option<GenerationCause>,
}

impl Default for BufferGenerationTracker {
    fn default() -> Self {
        Self {
            current: BufferGeneration::INITIAL,
            last_cause: None,
        }
    }
}

impl BufferGenerationTracker {
    /// Creates a tracker at the initial generation.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            current: BufferGeneration::INITIAL,
            last_cause: None,
        }
    }

    /// Returns the generation currently accepted by the active buffer.
    #[must_use]
    pub const fn current(&self) -> BufferGeneration {
        self.current
    }

    /// Returns the cause of the last invalidation, if any.
    #[must_use]
    pub const fn last_cause(&self) -> Option<GenerationCause> {
        self.last_cause
    }

    /// Retires the current buffer and returns its replacement generation.
    ///
    /// # Errors
    ///
    /// Returns [`SeekError::GenerationExhausted`] without changing state when
    /// the monotonic generation counter cannot advance.
    pub fn invalidate(&mut self, cause: GenerationCause) -> Result<BufferGeneration, SeekError> {
        let next = self.current.next()?;
        self.current = next;
        self.last_cause = Some(cause);
        Ok(next)
    }

    /// Returns whether a tagged result belongs to the active buffer.
    #[must_use]
    pub fn accepts(&self, generation: BufferGeneration) -> bool {
        self.current == generation
    }
}

/// A value tagged with the buffer generation that produced it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GenerationTagged<T> {
    generation: BufferGeneration,
    value: T,
}

impl<T> GenerationTagged<T> {
    /// Tags `value` with `generation`.
    #[must_use]
    pub const fn new(generation: BufferGeneration, value: T) -> Self {
        Self { generation, value }
    }

    /// Returns the producing generation.
    #[must_use]
    pub const fn generation(&self) -> BufferGeneration {
        self.generation
    }

    /// Borrows the tagged value without changing its generation.
    #[must_use]
    pub const fn value(&self) -> &T {
        &self.value
    }

    /// Consumes the value only when it belongs to `current`.
    ///
    /// # Errors
    ///
    /// Returns [`SeekError::StaleGeneration`] when this value was produced by
    /// a retired buffer generation.
    pub fn into_current(self, current: BufferGeneration) -> Result<T, SeekError> {
        if self.generation != current {
            return Err(SeekError::StaleGeneration {
                expected: current.value(),
                actual: self.generation.value(),
            });
        }
        Ok(self.value)
    }
}

/// A format-neutral seek target. Codec implementations decide how media units
/// map to source bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SeekTarget {
    /// An encoded source byte offset.
    ByteOffset(u64),
    /// A decoded media-frame position.
    Frame(u64),
    /// A decoded media timestamp in microseconds.
    Microseconds(u64),
}

/// Decoder delay and encoder-padding facts handed across a seek boundary.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DecoderDelayPadding {
    decoder_delay_frames: u64,
    encoder_padding_frames: u64,
}

impl DecoderDelayPadding {
    /// Creates delay/padding metadata without assuming a particular codec.
    #[must_use]
    pub const fn new(decoder_delay_frames: u64, encoder_padding_frames: u64) -> Self {
        Self {
            decoder_delay_frames,
            encoder_padding_frames,
        }
    }

    /// Returns decoder priming delay in output frames.
    #[must_use]
    pub const fn decoder_delay_frames(self) -> u64 {
        self.decoder_delay_frames
    }

    /// Returns encoder padding to trim at end of stream.
    #[must_use]
    pub const fn encoder_padding_frames(self) -> u64 {
        self.encoder_padding_frames
    }
}

/// Metadata returned by a decoder after a successful seek/reopen.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SeekMetadata {
    delay_padding: DecoderDelayPadding,
    source_position: Option<u64>,
}

impl SeekMetadata {
    /// Creates metadata with optional encoded-source position.
    #[must_use]
    pub const fn new(delay_padding: DecoderDelayPadding, source_position: Option<u64>) -> Self {
        Self {
            delay_padding,
            source_position,
        }
    }

    /// Returns decoder delay and encoder padding.
    #[must_use]
    pub const fn delay_padding(self) -> DecoderDelayPadding {
        self.delay_padding
    }

    /// Returns the source position reported by the decoder, if known.
    #[must_use]
    pub const fn source_position(self) -> Option<u64> {
        self.source_position
    }
}

/// A seek operation whose result is valid only for `next_generation`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SeekPlan {
    request_id: u64,
    target: SeekTarget,
    previous_generation: BufferGeneration,
    next_generation: BufferGeneration,
    reopens_source: bool,
}

impl SeekPlan {
    /// Returns the caller request ID.
    #[must_use]
    pub const fn request_id(self) -> u64 {
        self.request_id
    }

    /// Returns the requested target.
    #[must_use]
    pub const fn target(self) -> SeekTarget {
        self.target
    }

    /// Returns the generation retired when the plan began.
    #[must_use]
    pub const fn previous_generation(self) -> BufferGeneration {
        self.previous_generation
    }

    /// Returns the generation that a successful result must carry.
    #[must_use]
    pub const fn next_generation(self) -> BufferGeneration {
        self.next_generation
    }

    /// Returns whether source reopen is required before decoder seek.
    #[must_use]
    pub const fn reopens_source(self) -> bool {
        self.reopens_source
    }
}

/// The committed result of a worker-side seek.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SeekResult {
    request_id: u64,
    target: SeekTarget,
    generation: BufferGeneration,
    metadata: SeekMetadata,
    discontinuity: bool,
}

impl SeekResult {
    /// Returns the request ID that committed.
    #[must_use]
    pub const fn request_id(self) -> u64 {
        self.request_id
    }

    /// Returns the target that committed.
    #[must_use]
    pub const fn target(self) -> SeekTarget {
        self.target
    }

    /// Returns the new active buffer generation.
    #[must_use]
    pub const fn generation(self) -> BufferGeneration {
        self.generation
    }

    /// Returns decoder delay/padding handoff metadata.
    #[must_use]
    pub const fn metadata(self) -> SeekMetadata {
        self.metadata
    }

    /// Always returns true for a committed seek/reopen result.
    #[must_use]
    pub const fn discontinuity(self) -> bool {
        self.discontinuity
    }
}

/// Serializes worker-side seek/reopen operations and rejects stale results.
#[derive(Debug, Default)]
pub struct SeekCoordinator {
    generations: BufferGenerationTracker,
    active: Option<SeekPlan>,
}

impl SeekCoordinator {
    /// Creates a coordinator at the initial generation.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns the generation currently accepted by downstream buffers.
    #[must_use]
    pub const fn generation(&self) -> BufferGeneration {
        self.generations.current()
    }

    /// Returns the most recent invalidation cause.
    #[must_use]
    pub const fn last_cause(&self) -> Option<GenerationCause> {
        self.generations.last_cause()
    }

    /// Starts a seek and immediately retires the old buffer generation.
    ///
    /// # Errors
    ///
    /// Returns [`SeekError::GenerationExhausted`] without installing a plan
    /// when the buffer generation counter cannot advance.
    pub fn begin_seek(
        &mut self,
        request_id: u64,
        target: SeekTarget,
    ) -> Result<SeekPlan, SeekError> {
        self.begin(request_id, target, false, GenerationCause::Seek)
    }

    /// Starts a seek that must reopen the source before rebuilding decoder
    /// state.
    ///
    /// # Errors
    ///
    /// Returns [`SeekError::GenerationExhausted`] without installing a plan
    /// when the buffer generation counter cannot advance.
    pub fn begin_reopen(
        &mut self,
        request_id: u64,
        target: SeekTarget,
    ) -> Result<SeekPlan, SeekError> {
        self.begin(request_id, target, true, GenerationCause::Reopen)
    }

    /// Invalidates the active generation for a non-seek runtime event.
    ///
    /// # Errors
    ///
    /// Returns [`SeekError::GenerationExhausted`] without changing the active
    /// plan or generation when the counter cannot advance.
    pub fn invalidate(&mut self, cause: GenerationCause) -> Result<BufferGeneration, SeekError> {
        let next = self.generations.invalidate(cause)?;
        self.active = None;
        Ok(next)
    }

    /// Commits a plan only if it is still the active plan and generation.
    ///
    /// # Errors
    ///
    /// Returns [`SeekError::StaleRequest`] when another request superseded the
    /// plan, or [`SeekError::StaleGeneration`] when its generation retired.
    pub fn commit(
        &mut self,
        plan: SeekPlan,
        metadata: SeekMetadata,
    ) -> Result<SeekResult, SeekError> {
        if self.active != Some(plan) {
            return Err(SeekError::StaleRequest {
                request_id: plan.request_id,
            });
        }
        let current = self.generations.current();
        if current != plan.next_generation {
            return Err(SeekError::StaleGeneration {
                expected: current.value(),
                actual: plan.next_generation.value(),
            });
        }
        self.active = None;
        Ok(SeekResult {
            request_id: plan.request_id,
            target: plan.target,
            generation: plan.next_generation,
            metadata,
            discontinuity: true,
        })
    }

    /// Cancels a pending plan while keeping its invalidated generation.
    ///
    /// # Errors
    ///
    /// Returns [`SeekError::StaleRequest`] when the plan is no longer active.
    pub fn cancel(&mut self, plan: SeekPlan) -> Result<(), SeekError> {
        if self.active != Some(plan) {
            return Err(SeekError::StaleRequest {
                request_id: plan.request_id,
            });
        }
        self.active = None;
        Ok(())
    }

    /// Returns whether a generation-tagged result can be consumed.
    #[must_use]
    pub fn accepts(&self, generation: BufferGeneration) -> bool {
        self.generations.accepts(generation)
    }

    fn begin(
        &mut self,
        request_id: u64,
        target: SeekTarget,
        reopens_source: bool,
        cause: GenerationCause,
    ) -> Result<SeekPlan, SeekError> {
        let previous_generation = self.generations.current();
        let next_generation = self.generations.invalidate(cause)?;
        let plan = SeekPlan {
            request_id,
            target,
            previous_generation,
            next_generation,
            reopens_source,
        };
        self.active = Some(plan);
        Ok(plan)
    }
}

#[cfg(test)]
mod tests {
    use super::{
        BufferGeneration, BufferGenerationTracker, DecoderDelayPadding, GenerationCause,
        GenerationTagged, SeekCoordinator, SeekMetadata, SeekTarget,
    };
    use crate::errors::SeekError;

    #[test]
    fn seek_immediately_retires_old_generation_and_commits_metadata() {
        let mut coordinator = SeekCoordinator::new();
        let plan = coordinator
            .begin_reopen(7, SeekTarget::Microseconds(12_000))
            .expect("generation should advance");
        assert_eq!(plan.previous_generation(), BufferGeneration::INITIAL);
        assert_eq!(plan.next_generation().value(), 1);
        assert!(plan.reopens_source());
        assert!(coordinator.accepts(plan.next_generation()));
        let metadata = SeekMetadata::new(DecoderDelayPadding::new(5, 9), Some(44));
        let result = coordinator.commit(plan, metadata).expect("plan commits");
        assert!(result.discontinuity());
        assert_eq!(result.generation().value(), 1);
        assert_eq!(result.metadata().delay_padding().decoder_delay_frames(), 5);
        assert_eq!(result.metadata().source_position(), Some(44));
    }

    #[test]
    fn a_new_seek_makes_the_older_worker_result_stale() {
        let mut coordinator = SeekCoordinator::new();
        let first = coordinator
            .begin_seek(1, SeekTarget::Frame(10))
            .expect("first seek");
        let second = coordinator
            .begin_seek(2, SeekTarget::Frame(20))
            .expect("second seek");
        assert!(matches!(
            coordinator.commit(first, SeekMetadata::default()),
            Err(SeekError::StaleRequest { request_id: 1 })
        ));
        assert_eq!(
            coordinator
                .commit(second, SeekMetadata::default())
                .unwrap()
                .request_id(),
            2
        );
    }

    #[test]
    fn tagged_old_audio_cannot_enter_the_current_generation() {
        let mut tracker = BufferGenerationTracker::new();
        let old = GenerationTagged::new(tracker.current(), 11_u32);
        let current = tracker
            .invalidate(GenerationCause::Seek)
            .expect("generation should advance");
        assert!(matches!(
            old.into_current(current),
            Err(SeekError::StaleGeneration {
                expected: 1,
                actual: 0
            })
        ));
        let fresh = GenerationTagged::new(current, 12_u32);
        assert_eq!(fresh.into_current(current), Ok(12));
    }

    #[test]
    fn invalidation_causes_are_observable_and_counter_does_not_wrap() {
        let mut tracker = BufferGenerationTracker::new();
        tracker
            .invalidate(GenerationCause::UnderrunRecovery)
            .expect("first invalidation");
        assert_eq!(
            tracker.last_cause(),
            Some(GenerationCause::UnderrunRecovery)
        );
        let mut exhausted = BufferGenerationTracker {
            current: BufferGeneration::new(u64::MAX),
            last_cause: None,
        };
        assert_eq!(
            exhausted.invalidate(GenerationCause::Reopen),
            Err(SeekError::GenerationExhausted)
        );
        assert_eq!(exhausted.current().value(), u64::MAX);
    }

    #[test]
    fn cancellation_removes_active_plan_without_reusing_old_generation() {
        let mut coordinator = SeekCoordinator::new();
        let plan = coordinator
            .begin_seek(4, SeekTarget::ByteOffset(8))
            .expect("seek begins");
        coordinator.cancel(plan).expect("cancel succeeds");
        assert!(matches!(
            coordinator.commit(plan, SeekMetadata::default()),
            Err(SeekError::StaleRequest { request_id: 4 })
        ));
        assert_eq!(coordinator.generation().value(), 1);
    }
}
