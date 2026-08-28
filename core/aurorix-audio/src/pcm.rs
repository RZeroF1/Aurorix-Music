//! Interleaved `f32` PCM values and fixed-capacity audio blocks.
//!
//! [`AudioBlock`] allocates its storage only during setup. Samples are held as
//! atomic `f32` bit patterns so a prepared block can cross the SPSC boundary
//! without a lock or `unsafe` interior mutability. A producer owns a block
//! while it writes; a realtime consumer owns it while it reads. The buffer
//! state machine in `buffer.rs` establishes that ordering.

use std::{
    error::Error,
    fmt,
    sync::atomic::{AtomicU32, AtomicU64, AtomicUsize, Ordering},
};

use super::format::PcmFormat;

/// A fixed-capacity interleaved `f32` PCM block.
#[derive(Debug)]
pub struct AudioBlock {
    format: PcmFormat,
    capacity_frames: usize,
    valid_frames: AtomicUsize,
    generation: AtomicU64,
    samples: Vec<AtomicU32>,
}

impl AudioBlock {
    /// Allocates a zeroed block for the supplied PCM format.
    ///
    /// Allocation is intentionally performed here, outside realtime
    /// processing. The vector's length and capacity are fixed for the
    /// lifetime of the block.
    ///
    /// # Errors
    ///
    /// Returns [`PcmError::ZeroCapacity`] for an empty block or
    /// [`PcmError::SampleCountOverflow`] if the requested storage cannot be
    /// represented by `usize`.
    pub fn new(format: PcmFormat, capacity_frames: usize) -> Result<Self, PcmError> {
        if capacity_frames == 0 {
            return Err(PcmError::ZeroCapacity);
        }
        let sample_count = format
            .sample_count_for_frames(capacity_frames)
            .ok_or(PcmError::SampleCountOverflow)?;
        let samples = (0..sample_count).map(|_| AtomicU32::new(0)).collect();
        Ok(Self {
            format,
            capacity_frames,
            valid_frames: AtomicUsize::new(0),
            generation: AtomicU64::new(0),
            samples,
        })
    }

    /// Returns the stream format.
    #[must_use]
    pub const fn format(&self) -> PcmFormat {
        self.format
    }

    /// Returns the fixed number of frames this block can hold.
    #[must_use]
    pub const fn capacity_frames(&self) -> usize {
        self.capacity_frames
    }

    /// Returns the number of frames currently containing valid samples.
    #[must_use]
    pub fn valid_frames(&self) -> usize {
        self.valid_frames.load(Ordering::Acquire)
    }

    /// Returns the number of valid interleaved samples.
    #[must_use]
    pub fn valid_sample_count(&self) -> usize {
        self.valid_frames() * usize::from(self.format.channels())
    }

    /// Returns the generation assigned by the active buffer pipeline.
    #[must_use]
    pub fn generation(&self) -> u64 {
        self.generation.load(Ordering::Acquire)
    }

    /// Returns whether no valid frames are present.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.valid_frames() == 0
    }

    /// Returns whether all block frames are valid.
    #[must_use]
    pub fn is_full(&self) -> bool {
        self.valid_frames() == self.capacity_frames
    }

    /// Returns the number of preallocated interleaved sample cells.
    #[must_use]
    pub fn storage_sample_capacity(&self) -> usize {
        self.samples.len()
    }

    /// Reads one valid interleaved sample by index.
    ///
    /// `None` means the requested index is outside the valid sample range.
    #[must_use]
    pub fn sample_at(&self, sample_index: usize) -> Option<f32> {
        (sample_index < self.valid_sample_count())
            .then(|| f32::from_bits(self.samples[sample_index].load(Ordering::Relaxed)))
    }

    /// Assigns a generation before a producer starts filling the block.
    pub fn set_generation(&self, generation: u64) {
        self.generation.store(generation, Ordering::Release);
    }

    /// Sets the number of valid frames after direct storage filling.
    ///
    /// # Errors
    ///
    /// Returns [`PcmError::FrameCountExceedsCapacity`] when `frames` is too
    /// large for this block.
    pub fn set_valid_frames(&self, frames: usize) -> Result<(), PcmError> {
        if frames > self.capacity_frames {
            return Err(PcmError::FrameCountExceedsCapacity {
                requested: frames,
                capacity: self.capacity_frames,
            });
        }
        self.valid_frames.store(frames, Ordering::Release);
        Ok(())
    }

    /// Replaces the valid samples with an interleaved frame slice.
    ///
    /// The method writes only preallocated atomic cells. It performs no heap
    /// allocation and is intended for a Worker-held producer lease, never the
    /// callback.
    ///
    /// # Errors
    ///
    /// Returns an error when the sample count is not a whole number of frames
    /// or does not fit in this block.
    pub fn write_interleaved(&self, samples: &[f32]) -> Result<usize, PcmError> {
        let channels = usize::from(self.format.channels());
        if !samples.len().is_multiple_of(channels) {
            return Err(PcmError::SampleCountNotFrameAligned {
                samples: samples.len(),
                channels: self.format.channels(),
            });
        }
        let frames = samples.len() / channels;
        if frames > self.capacity_frames {
            return Err(PcmError::FrameCountExceedsCapacity {
                requested: frames,
                capacity: self.capacity_frames,
            });
        }

        for (destination, source) in self.samples.iter().zip(samples) {
            destination.store(source.to_bits(), Ordering::Relaxed);
        }
        self.valid_frames.store(frames, Ordering::Release);
        Ok(frames)
    }

    /// Copies valid frames starting at `frame_offset` into an interleaved
    /// destination and returns the number of complete frames copied.
    ///
    /// This is the callback-safe read operation: it uses only bounded slice
    /// iteration and atomic loads. The destination must already be supplied by
    /// the output adapter.
    ///
    /// # Errors
    ///
    /// Returns [`PcmError::SampleCountNotFrameAligned`] when the destination
    /// is not aligned to the channel count.
    pub fn copy_frames_to(
        &self,
        frame_offset: usize,
        destination: &mut [f32],
    ) -> Result<usize, PcmError> {
        let channels = usize::from(self.format.channels());
        if !destination.len().is_multiple_of(channels) {
            return Err(PcmError::SampleCountNotFrameAligned {
                samples: destination.len(),
                channels: self.format.channels(),
            });
        }

        let available_frames = self.valid_frames().saturating_sub(frame_offset);
        let frames = available_frames.min(destination.len() / channels);
        let source_start = frame_offset * channels;
        let sample_count = frames * channels;
        for (destination, source) in destination[..sample_count]
            .iter_mut()
            .zip(&self.samples[source_start..source_start + sample_count])
        {
            *destination = f32::from_bits(source.load(Ordering::Relaxed));
        }
        Ok(frames)
    }

    /// Copies all valid samples into an aligned destination.
    ///
    /// This convenience method is useful for deterministic Worker-side tests.
    ///
    /// # Errors
    ///
    /// Returns [`PcmError::SampleCountNotFrameAligned`] when the destination
    /// does not contain a whole number of interleaved frames.
    pub fn copy_to(&self, destination: &mut [f32]) -> Result<usize, PcmError> {
        self.copy_frames_to(0, destination)
    }

    /// Resets validity and assigns a new pipeline generation without changing
    /// the preallocated storage.
    pub fn reset(&self, generation: u64) {
        self.valid_frames.store(0, Ordering::Relaxed);
        self.generation.store(generation, Ordering::Release);
    }

    /// Clears only the currently valid sample range and resets it to empty.
    ///
    /// This operation is for Worker/coordinator cleanup, not a callback
    /// requirement. Normal recycling simply calls [`Self::reset`].
    pub fn clear_valid_samples(&self) {
        let sample_count = self.valid_sample_count();
        for sample in &self.samples[..sample_count] {
            sample.store(0, Ordering::Relaxed);
        }
        self.valid_frames.store(0, Ordering::Release);
    }
}

/// Errors produced while creating or filling an [`AudioBlock`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PcmError {
    /// A zero-frame block cannot participate in the bounded pipeline.
    ZeroCapacity,
    /// The requested interleaved storage overflowed `usize`.
    SampleCountOverflow,
    /// The supplied frame count exceeds the block capacity.
    FrameCountExceedsCapacity {
        /// Number of frames requested by the caller.
        requested: usize,
        /// Maximum frames available in the block.
        capacity: usize,
    },
    /// A sample slice is not divisible by the channel count.
    SampleCountNotFrameAligned {
        /// Number of samples supplied.
        samples: usize,
        /// Channel count used for alignment.
        channels: u8,
    },
}

impl fmt::Display for PcmError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ZeroCapacity => formatter.write_str("audio block capacity must be positive"),
            Self::SampleCountOverflow => {
                formatter.write_str("audio block sample count overflowed usize")
            }
            Self::FrameCountExceedsCapacity {
                requested,
                capacity,
            } => write!(
                formatter,
                "audio block received {requested} frames but capacity is {capacity}"
            ),
            Self::SampleCountNotFrameAligned { samples, channels } => write!(
                formatter,
                "sample slice of {samples} values is not aligned to {channels} channels"
            ),
        }
    }
}

impl Error for PcmError {}

#[cfg(test)]
mod tests {
    use super::{AudioBlock, PcmError};
    use crate::format::PcmFormat;

    fn stereo() -> PcmFormat {
        PcmFormat::new(48_000, 2).expect("test format is valid")
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
    fn block_has_fixed_storage_and_tracks_valid_frames() {
        let block = AudioBlock::new(stereo(), 4).expect("block is valid");
        assert_eq!(block.capacity_frames(), 4);
        assert!(block.is_empty());
        assert_eq!(block.storage_sample_capacity(), 8);

        assert_eq!(block.write_interleaved(&[0.25, -0.25, 0.5, -0.5]), Ok(2));
        assert_eq!(block.valid_frames(), 2);
        assert_eq!(block.sample_at(0), Some(0.25));
        assert_eq!(block.sample_at(3), Some(-0.5));
        assert_eq!(block.sample_at(4), None);
        assert!(!block.is_full());
    }

    #[test]
    fn writes_reject_alignment_and_capacity_errors() {
        let block = AudioBlock::new(stereo(), 2).expect("block is valid");
        assert_eq!(
            block.write_interleaved(&[1.0]),
            Err(PcmError::SampleCountNotFrameAligned {
                samples: 1,
                channels: 2,
            })
        );
        assert_eq!(
            block.write_interleaved(&[0.0; 6]),
            Err(PcmError::FrameCountExceedsCapacity {
                requested: 3,
                capacity: 2,
            })
        );
    }

    #[test]
    fn reset_reuses_fixed_storage_and_changes_generation() {
        let block = AudioBlock::new(stereo(), 2).expect("block is valid");
        block
            .write_interleaved(&[1.0, 2.0, 3.0, 4.0])
            .expect("samples fit");
        block.set_generation(7);
        assert_eq!(block.generation(), 7);
        assert_eq!(block.valid_sample_count(), 4);

        block.reset(8);
        assert_eq!(block.generation(), 8);
        assert!(block.is_empty());
        assert_eq!(block.storage_sample_capacity(), 4);
    }

    #[test]
    fn copy_to_is_bounded_by_destination_frames() {
        let block = AudioBlock::new(stereo(), 3).expect("block is valid");
        block
            .write_interleaved(&[1.0, 2.0, 3.0, 4.0, 5.0, 6.0])
            .expect("samples fit");
        let mut destination = [0.0; 4];
        assert_eq!(block.copy_to(&mut destination), Ok(2));
        assert_sample_bits_eq(&destination, &[1.0, 2.0, 3.0, 4.0]);
    }
}
