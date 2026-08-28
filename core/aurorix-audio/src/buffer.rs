//! Preallocated, bounded single-producer/single-consumer audio handoff.
//!
//! The pool is created before rendering begins. A decoder or DSP worker owns
//! an [`AudioProducer`]; a realtime callback owns a consumer from
//! `realtime.rs`. Slots move only through `vacant -> writing -> ready ->
//! reading -> vacant`. All slot coordination uses atomics, and neither side
//! needs a mutex, channel, allocation, or blocking wait while handling audio.

use std::{
    error::Error,
    fmt,
    mem::size_of,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU8, AtomicU32, AtomicU64, AtomicUsize, Ordering},
    },
};

use super::{
    format::PcmFormat,
    pcm::{AudioBlock, PcmError},
};

/// Minimum useful number of block slots in a bounded audio buffer.
pub const MIN_BUFFER_SLOTS: usize = 2;

/// Largest permitted number of preallocated SPSC slots.
pub const MAX_BUFFER_SLOTS: usize = 256;

/// Largest permitted number of frames in one preallocated block.
pub const MAX_BLOCK_FRAMES: usize = 16_384;

/// Largest permitted sample-storage allocation for one audio buffer.
pub const MAX_BUFFER_BYTES: usize = 64 * 1024 * 1024;

const SLOT_VACANT: u8 = 0;
const SLOT_WRITING: u8 = 1;
const SLOT_READY: u8 = 2;
const SLOT_READING: u8 = 3;

/// A fixed-capacity SPSC pool of reusable [`AudioBlock`] values.
#[derive(Debug)]
pub struct AudioBuffer {
    format: PcmFormat,
    slots: Box<[BufferSlot]>,
    capacity_frames_per_block: usize,
    generation: AtomicU64,
    resetting: AtomicBool,
    endpoint_entries: AtomicUsize,
}

#[derive(Debug)]
struct BufferSlot {
    state: AtomicU8,
    block: AudioBlock,
}

impl AudioBuffer {
    /// Allocates a bounded pool with `slot_count` reusable blocks.
    ///
    /// The returned [`Arc`] is cloned while endpoints are created, never by
    /// the callback. Every block and atomic slot state is allocated before
    /// steady-state operation begins.
    ///
    /// # Errors
    ///
    /// Returns a typed capacity or PCM error before allocation when the request
    /// is outside the fixed buffer limits.
    pub fn new(
        format: PcmFormat,
        slot_count: usize,
        capacity_frames_per_block: usize,
    ) -> Result<Arc<Self>, BufferError> {
        if slot_count < MIN_BUFFER_SLOTS {
            return Err(BufferError::TooFewSlots { actual: slot_count });
        }
        if slot_count > MAX_BUFFER_SLOTS {
            return Err(BufferError::TooManySlots { actual: slot_count });
        }
        if capacity_frames_per_block == 0 {
            return Err(BufferError::Pcm(PcmError::ZeroCapacity));
        }
        if capacity_frames_per_block > MAX_BLOCK_FRAMES {
            return Err(BufferError::TooManyFramesPerBlock {
                actual: capacity_frames_per_block,
            });
        }
        let samples_per_block = format
            .sample_count_for_frames(capacity_frames_per_block)
            .ok_or(BufferError::Pcm(PcmError::SampleCountOverflow))?;
        let requested_bytes = samples_per_block
            .checked_mul(size_of::<AtomicU32>())
            .and_then(|bytes_per_block| bytes_per_block.checked_mul(slot_count))
            .unwrap_or(usize::MAX);
        if requested_bytes > MAX_BUFFER_BYTES {
            return Err(BufferError::MemoryLimitExceeded { requested_bytes });
        }

        let mut slots = Vec::with_capacity(slot_count);
        for _ in 0..slot_count {
            slots.push(BufferSlot {
                state: AtomicU8::new(SLOT_VACANT),
                block: AudioBlock::new(format, capacity_frames_per_block)?,
            });
        }
        Ok(Arc::new(Self {
            format,
            slots: slots.into_boxed_slice(),
            capacity_frames_per_block,
            generation: AtomicU64::new(0),
            resetting: AtomicBool::new(false),
            endpoint_entries: AtomicUsize::new(0),
        }))
    }

    /// Creates the only producer endpoint for this SPSC pool.
    ///
    /// Call this during graph setup. The type system prevents one producer
    /// endpoint from holding more than one writing lease at a time.
    #[must_use]
    pub fn producer(self: &Arc<Self>) -> AudioProducer {
        AudioProducer {
            buffer: Arc::clone(self),
            next_slot: 0,
        }
    }

    /// Returns the fixed PCM format for every block in this pool.
    #[must_use]
    pub const fn format(&self) -> PcmFormat {
        self.format
    }

    /// Returns the total number of slots allocated by this pool.
    #[must_use]
    pub fn slot_capacity(&self) -> usize {
        self.slots.len()
    }

    /// Returns the fixed frame capacity of every slot.
    #[must_use]
    pub const fn capacity_frames_per_block(&self) -> usize {
        self.capacity_frames_per_block
    }

    /// Returns the generation assigned to newly acquired blocks.
    #[must_use]
    pub fn generation(&self) -> u64 {
        self.generation.load(Ordering::Acquire)
    }

    /// Counts vacant slots for diagnostics outside the callback.
    #[must_use]
    pub fn vacant_slots(&self) -> usize {
        self.count_slots(SLOT_VACANT)
    }

    /// Counts ready slots for diagnostics outside the callback.
    #[must_use]
    pub fn ready_slots(&self) -> usize {
        self.count_slots(SLOT_READY)
    }

    /// Invalidates ready blocks and advances the pool generation.
    ///
    /// A coordinator must first stop the producer and callback from starting
    /// new work. The method rejects a live writing or reading slot rather than
    /// racing it. It is a worker/control-plane method and must never run in a
    /// realtime callback.
    ///
    /// # Errors
    ///
    /// Returns [`BufferGenerationError`] when reset coordination has not
    /// reached a quiescent point or when `generation` does not advance.
    pub fn try_advance_generation(&self, generation: u64) -> Result<(), BufferGenerationError> {
        let current = self.generation();
        if generation <= current {
            return Err(BufferGenerationError::GenerationNotAdvanced {
                current,
                requested: generation,
            });
        }
        if self
            .resetting
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return Err(BufferGenerationError::ResetInProgress);
        }
        if self.endpoint_entries.load(Ordering::Acquire) != 0 {
            self.resetting.store(false, Ordering::Release);
            return Err(BufferGenerationError::EndpointTransitionInProgress);
        }

        for (index, slot) in self.slots.iter().enumerate() {
            match slot.state.load(Ordering::Acquire) {
                SLOT_VACANT | SLOT_READY => {}
                state => {
                    self.resetting.store(false, Ordering::Release);
                    return Err(BufferGenerationError::SlotInUse {
                        index,
                        state: BufferSlotState::from_raw(state),
                    });
                }
            }
        }

        for slot in &self.slots {
            slot.block.reset(generation);
            slot.state.store(SLOT_VACANT, Ordering::Release);
        }
        self.generation.store(generation, Ordering::Release);
        self.resetting.store(false, Ordering::Release);
        Ok(())
    }

    fn count_slots(&self, state: u8) -> usize {
        self.slots
            .iter()
            .filter(|slot| slot.state.load(Ordering::Acquire) == state)
            .count()
    }

    fn slot(&self, index: usize) -> &BufferSlot {
        &self.slots[index]
    }

    fn try_begin_write(&self, index: usize) -> Option<u64> {
        if !self.try_enter_endpoint_transition() {
            return None;
        }
        let slot = self.slot(index);
        let acquired = slot
            .state
            .compare_exchange(
                SLOT_VACANT,
                SLOT_WRITING,
                Ordering::Acquire,
                Ordering::Relaxed,
            )
            .is_ok();
        self.leave_endpoint_transition();
        if !acquired {
            return None;
        }
        let generation = self.generation();
        slot.block.reset(generation);
        Some(generation)
    }

    fn publish(&self, index: usize, generation: u64) -> Result<(), PublishError> {
        let slot = self.slot(index);
        if self.generation() != generation || slot.block.generation() != generation {
            self.abort_write(index);
            return Err(PublishError::StaleGeneration {
                block: generation,
                current: self.generation(),
            });
        }
        if slot.block.is_empty() {
            self.abort_write(index);
            return Err(PublishError::EmptyBlock);
        }
        slot.state.store(SLOT_READY, Ordering::Release);
        Ok(())
    }

    fn abort_write(&self, index: usize) {
        let slot = self.slot(index);
        slot.block.reset(self.generation());
        slot.state.store(SLOT_VACANT, Ordering::Release);
    }

    pub(crate) fn try_begin_render(&self, index: usize) -> Option<ReadyBlock> {
        if !self.try_enter_endpoint_transition() {
            return None;
        }
        let slot = self.slot(index);
        let acquired = slot
            .state
            .compare_exchange(
                SLOT_READY,
                SLOT_READING,
                Ordering::Acquire,
                Ordering::Relaxed,
            )
            .is_ok();
        self.leave_endpoint_transition();
        if !acquired {
            return None;
        }
        Some(ReadyBlock {
            index,
            valid_frames: slot.block.valid_frames(),
            generation: slot.block.generation(),
        })
    }

    pub(crate) fn block_for_render(&self, index: usize) -> &AudioBlock {
        &self.slot(index).block
    }

    pub(crate) fn finish_render(&self, index: usize) {
        let slot = self.slot(index);
        slot.block.reset(self.generation());
        slot.state.store(SLOT_VACANT, Ordering::Release);
    }

    fn try_enter_endpoint_transition(&self) -> bool {
        if self.resetting.load(Ordering::Acquire) {
            return false;
        }
        self.endpoint_entries.fetch_add(1, Ordering::AcqRel);
        if self.resetting.load(Ordering::Acquire) {
            self.leave_endpoint_transition();
            return false;
        }
        true
    }

    fn leave_endpoint_transition(&self) {
        self.endpoint_entries.fetch_sub(1, Ordering::Release);
    }
}

/// The only worker-side producer for an [`AudioBuffer`].
#[derive(Debug)]
pub struct AudioProducer {
    buffer: Arc<AudioBuffer>,
    next_slot: usize,
}

impl AudioProducer {
    /// Attempts to acquire the next vacant fixed-capacity block.
    ///
    /// `None` is deterministic backpressure: the worker must wait or apply a
    /// policy outside the callback rather than allocating another block.
    pub fn try_acquire(&mut self) -> Option<ProducerLease<'_>> {
        let index = self.next_slot;
        let generation = self.buffer.try_begin_write(index)?;
        Some(ProducerLease {
            producer: self,
            index,
            generation,
            finished: false,
        })
    }

    /// Returns the shared pool used by this producer.
    #[must_use]
    pub fn buffer(&self) -> &Arc<AudioBuffer> {
        &self.buffer
    }

    fn advance_slot(&mut self) {
        self.next_slot = (self.next_slot + 1) % self.buffer.slot_capacity();
    }
}

/// An exclusive worker-side lease for one writing block.
#[derive(Debug)]
pub struct ProducerLease<'a> {
    producer: &'a mut AudioProducer,
    index: usize,
    generation: u64,
    finished: bool,
}

impl ProducerLease<'_> {
    /// Returns the fixed-capacity block held by this producer lease.
    #[must_use]
    pub fn block(&self) -> &AudioBlock {
        &self.producer.buffer.slot(self.index).block
    }

    /// Returns the generation captured when this lease was acquired.
    #[must_use]
    pub const fn generation(&self) -> u64 {
        self.generation
    }

    /// Writes an interleaved PCM slice into the preallocated block.
    ///
    /// # Errors
    ///
    /// Returns the block validation error without publishing partial work.
    pub fn write_interleaved(&self, samples: &[f32]) -> Result<usize, PcmError> {
        self.block().write_interleaved(samples)
    }

    /// Marks direct preallocated storage as containing `frames` valid frames.
    ///
    /// # Errors
    ///
    /// Returns a block validation error for an out-of-range count.
    pub fn set_valid_frames(&self, frames: usize) -> Result<(), PcmError> {
        self.block().set_valid_frames(frames)
    }

    /// Publishes this prepared block to the realtime consumer.
    ///
    /// # Errors
    ///
    /// Returns a typed error for a stale generation or empty block. A failed
    /// publish returns the slot to the bounded vacant pool.
    pub fn publish(mut self) -> Result<(), PublishError> {
        self.producer.buffer.publish(self.index, self.generation)?;
        self.producer.advance_slot();
        self.finished = true;
        Ok(())
    }
}

impl Drop for ProducerLease<'_> {
    fn drop(&mut self) {
        if !self.finished {
            self.producer.buffer.abort_write(self.index);
        }
    }
}

/// A block claimed by the realtime side of the SPSC ring.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ReadyBlock {
    /// Slot index retained by the realtime consumer.
    pub(crate) index: usize,
    /// Number of valid frames captured after producer publication.
    pub(crate) valid_frames: usize,
    /// Generation captured after producer publication.
    pub(crate) generation: u64,
}

/// Observable state used in control-plane generation-reset errors.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BufferSlotState {
    /// A worker currently owns the slot.
    Writing,
    /// The callback currently owns the slot.
    Reading,
    /// An unknown state value was observed; treat this as unsafe to reset.
    Unknown,
}

impl BufferSlotState {
    const fn from_raw(value: u8) -> Self {
        match value {
            SLOT_WRITING => Self::Writing,
            SLOT_READING => Self::Reading,
            _ => Self::Unknown,
        }
    }
}

/// Failure while constructing a bounded pool.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BufferError {
    /// The pool needs at least one vacant and one ready slot.
    TooFewSlots {
        /// Requested slot count.
        actual: usize,
    },
    /// The requested slot count exceeds the fixed SPSC pool limit.
    TooManySlots {
        /// Requested slot count.
        actual: usize,
    },
    /// The requested per-block frame capacity exceeds the fixed limit.
    TooManyFramesPerBlock {
        /// Requested frame capacity.
        actual: usize,
    },
    /// The requested preallocated sample storage exceeds the memory budget.
    MemoryLimitExceeded {
        /// Number of bytes required for atomic sample cells.
        requested_bytes: usize,
    },
    /// An underlying PCM block could not be created.
    Pcm(PcmError),
}

impl fmt::Display for BufferError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooFewSlots { actual } => write!(
                formatter,
                "audio buffer needs at least {MIN_BUFFER_SLOTS} slots, got {actual}"
            ),
            Self::TooManySlots { actual } => write!(
                formatter,
                "audio buffer permits at most {MAX_BUFFER_SLOTS} slots, got {actual}"
            ),
            Self::TooManyFramesPerBlock { actual } => write!(
                formatter,
                "audio block permits at most {MAX_BLOCK_FRAMES} frames, got {actual}"
            ),
            Self::MemoryLimitExceeded { requested_bytes } => write!(
                formatter,
                "audio buffer requires {requested_bytes} bytes; maximum is {MAX_BUFFER_BYTES}"
            ),
            Self::Pcm(error) => error.fmt(formatter),
        }
    }
}

impl Error for BufferError {}

impl From<PcmError> for BufferError {
    fn from(error: PcmError) -> Self {
        Self::Pcm(error)
    }
}

/// Failure while publishing a prepared producer lease.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PublishError {
    /// A generation reset made this block stale before publication.
    StaleGeneration {
        /// Generation held by the producer block.
        block: u64,
        /// Current pool generation.
        current: u64,
    },
    /// An empty block would create a meaningless callback transition.
    EmptyBlock,
}

impl fmt::Display for PublishError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::StaleGeneration { block, current } => write!(
                formatter,
                "audio block generation {block} is stale; pool generation is {current}"
            ),
            Self::EmptyBlock => formatter.write_str("cannot publish an empty audio block"),
        }
    }
}

impl Error for PublishError {}

/// Failure while a coordinator resets the pipeline generation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BufferGenerationError {
    /// The requested generation would not make stale blocks distinguishable.
    GenerationNotAdvanced {
        /// Current pool generation.
        current: u64,
        /// Generation requested by the coordinator.
        requested: u64,
    },
    /// Another coordinator already owns reset coordination.
    ResetInProgress,
    /// A producer or consumer was entering a slot transition when reset began.
    EndpointTransitionInProgress,
    /// A worker or callback has not reached a quiescent point.
    SlotInUse {
        /// Slot that prevented reset.
        index: usize,
        /// Operation that still owns the slot.
        state: BufferSlotState,
    },
}

impl fmt::Display for BufferGenerationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::GenerationNotAdvanced { current, requested } => write!(
                formatter,
                "audio buffer generation must advance beyond {current}, got {requested}"
            ),
            Self::ResetInProgress => {
                formatter.write_str("audio buffer reset is already in progress")
            }
            Self::EndpointTransitionInProgress => {
                formatter.write_str("audio buffer endpoint transition is still in progress")
            }
            Self::SlotInUse { index, state } => write!(
                formatter,
                "audio buffer slot {index} is still in use by {state:?}"
            ),
        }
    }
}

impl Error for BufferGenerationError {}

#[cfg(test)]
mod tests {
    use super::{
        AudioBuffer, BufferError, BufferGenerationError, MAX_BLOCK_FRAMES, MAX_BUFFER_BYTES,
        MAX_BUFFER_SLOTS, MIN_BUFFER_SLOTS, PublishError,
    };
    use crate::format::PcmFormat;

    fn buffer() -> std::sync::Arc<AudioBuffer> {
        AudioBuffer::new(PcmFormat::new(48_000, 2).expect("format is valid"), 3, 4)
            .expect("buffer is valid")
    }

    #[test]
    fn pool_is_bounded_and_reuses_published_slots() {
        let pool = buffer();
        let mut producer = pool.producer();
        assert_eq!(pool.slot_capacity(), 3);
        assert_eq!(pool.vacant_slots(), 3);

        let lease = producer.try_acquire().expect("one slot is vacant");
        lease
            .write_interleaved(&[1.0, 2.0, 3.0, 4.0])
            .expect("samples fit");
        lease.publish().expect("ready slot is available");
        assert_eq!(pool.vacant_slots(), 2);
        assert_eq!(pool.ready_slots(), 1);

        let ready = pool.try_begin_render(0).expect("published block exists");
        assert_eq!(ready.valid_frames, 2);
        pool.finish_render(ready.index);
        assert_eq!(pool.vacant_slots(), 3);
        assert_eq!(pool.ready_slots(), 0);
    }

    #[test]
    fn full_pool_applies_backpressure_without_growth() {
        let pool = buffer();
        let mut producer = pool.producer();
        for _ in 0..3 {
            let lease = producer.try_acquire().expect("slot is vacant");
            lease.set_valid_frames(1).expect("frame count fits");
            lease.publish().expect("slot publishes once");
        }
        assert!(producer.try_acquire().is_none());
        assert_eq!(pool.slot_capacity(), 3);
        assert_eq!(pool.ready_slots(), 3);
    }

    #[test]
    fn dropped_lease_returns_its_slot_without_publishing() {
        let pool = buffer();
        let mut producer = pool.producer();
        let lease = producer.try_acquire().expect("slot is vacant");
        lease.set_valid_frames(1).expect("frame count fits");
        drop(lease);
        assert_eq!(pool.vacant_slots(), 3);
        assert_eq!(pool.ready_slots(), 0);
    }

    #[test]
    fn empty_blocks_are_rejected_and_returned() {
        let pool = buffer();
        let mut producer = pool.producer();
        let error = producer
            .try_acquire()
            .expect("slot is vacant")
            .publish()
            .expect_err("empty block cannot be published");
        assert_eq!(error, PublishError::EmptyBlock);
        assert_eq!(pool.vacant_slots(), 3);
    }

    #[test]
    fn generation_reset_discards_ready_blocks_after_quiescence() {
        let pool = buffer();
        let mut producer = pool.producer();
        let lease = producer.try_acquire().expect("slot is vacant");
        lease.set_valid_frames(1).expect("frame count fits");
        lease.publish().expect("block publishes");

        pool.try_advance_generation(1)
            .expect("ready blocks can be discarded at quiescence");
        assert_eq!(pool.generation(), 1);
        assert_eq!(pool.ready_slots(), 0);
        assert_eq!(pool.vacant_slots(), 3);
    }

    #[test]
    fn generation_reset_rejects_a_live_producer_lease() {
        let pool = buffer();
        let mut producer = pool.producer();
        let lease = producer.try_acquire().expect("slot is vacant");
        assert!(matches!(
            pool.try_advance_generation(1),
            Err(BufferGenerationError::SlotInUse { .. })
        ));
        drop(lease);
    }

    #[test]
    fn pool_rejects_less_than_two_slots() {
        let error = AudioBuffer::new(
            PcmFormat::new(44_100, 2).expect("format is valid"),
            MIN_BUFFER_SLOTS - 1,
            4,
        )
        .expect_err("a producer and consumer need separate capacity");
        assert_eq!(error, BufferError::TooFewSlots { actual: 1 });
    }

    #[test]
    fn pool_rejects_dimensions_beyond_its_preallocation_limits() {
        let format = PcmFormat::new(44_100, 2).expect("format is valid");
        let slots_error = AudioBuffer::new(format, MAX_BUFFER_SLOTS + 1, 4)
            .expect_err("slot count must be bounded before allocation");
        assert_eq!(
            slots_error,
            BufferError::TooManySlots {
                actual: MAX_BUFFER_SLOTS + 1,
            }
        );
        let frame_error = AudioBuffer::new(format, 2, MAX_BLOCK_FRAMES + 1)
            .expect_err("block frame capacity must be bounded before allocation");
        assert_eq!(
            frame_error,
            BufferError::TooManyFramesPerBlock {
                actual: MAX_BLOCK_FRAMES + 1,
            }
        );

        let memory_error = AudioBuffer::new(
            PcmFormat::new(48_000, 8).expect("format is valid"),
            MAX_BUFFER_SLOTS,
            MAX_BLOCK_FRAMES,
        )
        .expect_err("memory budget must be checked before allocating slots");
        assert!(matches!(
            memory_error,
            BufferError::MemoryLimitExceeded { requested_bytes }
                if requested_bytes > MAX_BUFFER_BYTES
        ));
    }
}
