//! Deterministic, platform-neutral output capture for offline playback tests.
//!
//! The sink is a control/test-plane observer. It is intentionally bounded and
//! is not a hardware callback implementation. A caller supplies all storage
//! limits before capture begins; once a limit is reached, capture fails rather
//! than growing implicitly.

use std::{error::Error, fmt};

use crate::realtime::CompactClockSample;

/// Default maximum number of interleaved samples retained by a capture.
pub const DEFAULT_MAX_CAPTURE_SAMPLES: usize = 8 * 1024 * 1024;
/// Default maximum number of clock samples retained by a capture.
pub const DEFAULT_MAX_CLOCK_SAMPLES: usize = 1_000_000;
/// Default maximum number of discontinuities retained by a capture.
pub const DEFAULT_MAX_DISCONTINUITIES: usize = 4_096;

/// Why a captured output timeline was discontinuous.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CaptureDiscontinuityReason {
    /// A worker-side seek took effect.
    Seek,
    /// Output paused.
    Pause,
    /// Output resumed.
    Resume,
    /// The active source changed.
    SourceTransition,
    /// The output path was restarted.
    OutputRestart,
    /// An underrun recovery path was activated.
    UnderrunRecovery,
    /// A graph or format boundary changed.
    GraphRebuild,
}

impl CaptureDiscontinuityReason {
    const fn code(self) -> u8 {
        match self {
            Self::Seek => 1,
            Self::Pause => 2,
            Self::Resume => 3,
            Self::SourceTransition => 4,
            Self::OutputRestart => 5,
            Self::UnderrunRecovery => 6,
            Self::GraphRebuild => 7,
        }
    }
}

/// One captured discontinuity marker.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CaptureDiscontinuity {
    /// The epoch after the discontinuity took effect.
    pub epoch: u64,
    /// The control-plane reason for the boundary.
    pub reason: CaptureDiscontinuityReason,
}

/// Fixed capture limits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CaptureSinkConfig {
    pcm_samples: usize,
    clock_observations: usize,
    discontinuity_markers: usize,
}

impl CaptureSinkConfig {
    /// Creates fixed limits. Zero limits are valid and reject the first item.
    #[must_use]
    pub const fn new(
        max_samples: usize,
        max_clock_samples: usize,
        max_discontinuities: usize,
    ) -> Self {
        Self {
            pcm_samples: max_samples,
            clock_observations: max_clock_samples,
            discontinuity_markers: max_discontinuities,
        }
    }

    /// Returns the maximum interleaved samples.
    #[must_use]
    pub const fn max_samples(self) -> usize {
        self.pcm_samples
    }

    /// Returns the maximum clock samples.
    #[must_use]
    pub const fn max_clock_samples(self) -> usize {
        self.clock_observations
    }

    /// Returns the maximum discontinuity markers.
    #[must_use]
    pub const fn max_discontinuities(self) -> usize {
        self.discontinuity_markers
    }
}

impl Default for CaptureSinkConfig {
    fn default() -> Self {
        Self::new(
            DEFAULT_MAX_CAPTURE_SAMPLES,
            DEFAULT_MAX_CLOCK_SAMPLES,
            DEFAULT_MAX_DISCONTINUITIES,
        )
    }
}

/// Stable 256-bit digest of all captured observations.
///
/// This is a deterministic evidence digest, not a cryptographic identity or a
/// replacement for the Sync operation SHA-256 contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CaptureDigest([u8; 32]);

impl CaptureDigest {
    /// Returns the digest bytes in stable big-endian lane order.
    #[must_use]
    pub const fn as_bytes(self) -> [u8; 32] {
        self.0
    }

    /// Returns lowercase hexadecimal without allocating internally.
    #[must_use]
    pub fn to_hex(self) -> String {
        const HEX: &[u8; 16] = b"0123456789abcdef";
        let mut output = String::with_capacity(64);
        for byte in self.0 {
            output.push(HEX[usize::from(byte >> 4)] as char);
            output.push(HEX[usize::from(byte & 0x0f)] as char);
        }
        output
    }
}

impl fmt::Display for CaptureDigest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.to_hex())
    }
}

/// A bounded deterministic capture sink.
#[derive(Debug, Clone, PartialEq)]
pub struct CaptureSink {
    config: CaptureSinkConfig,
    channels: u8,
    samples: Vec<f32>,
    clock_samples: Vec<CompactClockSample>,
    discontinuities: Vec<CaptureDiscontinuity>,
}

impl CaptureSink {
    /// Creates an empty capture for interleaved output.
    ///
    /// # Errors
    ///
    /// Returns `CaptureSinkError::InvalidChannels` for zero channels.
    pub fn new(channels: u8, config: CaptureSinkConfig) -> Result<Self, CaptureSinkError> {
        if channels == 0 {
            return Err(CaptureSinkError::InvalidChannels);
        }
        Ok(Self {
            config,
            channels,
            samples: Vec::new(),
            clock_samples: Vec::new(),
            discontinuities: Vec::new(),
        })
    }

    /// Returns the immutable capture configuration.
    #[must_use]
    pub const fn config(&self) -> CaptureSinkConfig {
        self.config
    }

    /// Returns the interleaved channel count.
    #[must_use]
    pub const fn channels(&self) -> u8 {
        self.channels
    }

    /// Returns all captured interleaved PCM samples.
    #[must_use]
    pub fn samples(&self) -> &[f32] {
        &self.samples
    }

    /// Returns all captured compact clock observations.
    #[must_use]
    pub fn clock_samples(&self) -> &[CompactClockSample] {
        &self.clock_samples
    }

    /// Returns all captured discontinuity markers.
    #[must_use]
    pub fn discontinuities(&self) -> &[CaptureDiscontinuity] {
        &self.discontinuities
    }

    /// Returns the number of captured output frames.
    #[must_use]
    pub fn captured_frames(&self) -> usize {
        self.samples.len() / usize::from(self.channels)
    }

    /// Appends frame-aligned interleaved PCM.
    ///
    /// # Errors
    ///
    /// Returns a bounded-capacity or alignment error without changing the
    /// capture.
    pub fn capture_pcm(&mut self, samples: &[f32]) -> Result<(), CaptureSinkError> {
        let channels = usize::from(self.channels);
        if !samples.len().is_multiple_of(channels) {
            return Err(CaptureSinkError::MisalignedSamples {
                samples: samples.len(),
                channels: self.channels,
            });
        }
        let required = self
            .samples
            .len()
            .checked_add(samples.len())
            .ok_or(CaptureSinkError::CapacityOverflow)?;
        if required > self.config.max_samples() {
            return Err(CaptureSinkError::CapacityExceeded {
                kind: CaptureLimitKind::PcmSamples,
                requested: required,
                maximum: self.config.max_samples(),
            });
        }
        self.samples.extend_from_slice(samples);
        Ok(())
    }

    /// Appends one callback clock observation.
    ///
    /// # Errors
    ///
    /// Returns a bounded-capacity error without changing the capture.
    pub fn capture_clock(&mut self, sample: CompactClockSample) -> Result<(), CaptureSinkError> {
        let required = self
            .clock_samples
            .len()
            .checked_add(1)
            .ok_or(CaptureSinkError::CapacityOverflow)?;
        if required > self.config.max_clock_samples() {
            return Err(CaptureSinkError::CapacityExceeded {
                kind: CaptureLimitKind::ClockSamples,
                requested: required,
                maximum: self.config.max_clock_samples(),
            });
        }
        self.clock_samples.push(sample);
        Ok(())
    }

    /// Records one epoch/discontinuity marker.
    ///
    /// Repeating the same marker is retained because the capture is an
    /// observation of the actual sequence, not a set of unique states.
    ///
    /// # Errors
    ///
    /// Returns a bounded-capacity error without changing the capture.
    pub fn capture_discontinuity(
        &mut self,
        discontinuity: CaptureDiscontinuity,
    ) -> Result<(), CaptureSinkError> {
        let required = self
            .discontinuities
            .len()
            .checked_add(1)
            .ok_or(CaptureSinkError::CapacityOverflow)?;
        if required > self.config.max_discontinuities() {
            return Err(CaptureSinkError::CapacityExceeded {
                kind: CaptureLimitKind::Discontinuities,
                requested: required,
                maximum: self.config.max_discontinuities(),
            });
        }
        self.discontinuities.push(discontinuity);
        Ok(())
    }

    /// Captures a complete output observation in a fixed operation order.
    ///
    /// If one append fails, all capacity checks run before mutation, so the
    /// sink remains unchanged.
    ///
    /// # Errors
    ///
    /// Returns the first capacity or alignment error.
    pub fn capture_observation(
        &mut self,
        samples: &[f32],
        clock: CompactClockSample,
        discontinuity: Option<CaptureDiscontinuity>,
    ) -> Result<(), CaptureSinkError> {
        let channels = usize::from(self.channels);
        if !samples.len().is_multiple_of(channels) {
            return Err(CaptureSinkError::MisalignedSamples {
                samples: samples.len(),
                channels: self.channels,
            });
        }
        let sample_count = self
            .samples
            .len()
            .checked_add(samples.len())
            .ok_or(CaptureSinkError::CapacityOverflow)?;
        if sample_count > self.config.max_samples() {
            return Err(CaptureSinkError::CapacityExceeded {
                kind: CaptureLimitKind::PcmSamples,
                requested: sample_count,
                maximum: self.config.max_samples(),
            });
        }
        let clock_count = self
            .clock_samples
            .len()
            .checked_add(1)
            .ok_or(CaptureSinkError::CapacityOverflow)?;
        if clock_count > self.config.max_clock_samples() {
            return Err(CaptureSinkError::CapacityExceeded {
                kind: CaptureLimitKind::ClockSamples,
                requested: clock_count,
                maximum: self.config.max_clock_samples(),
            });
        }
        if let Some(marker) = discontinuity {
            let marker_count = self
                .discontinuities
                .len()
                .checked_add(1)
                .ok_or(CaptureSinkError::CapacityOverflow)?;
            if marker_count > self.config.max_discontinuities() {
                return Err(CaptureSinkError::CapacityExceeded {
                    kind: CaptureLimitKind::Discontinuities,
                    requested: marker_count,
                    maximum: self.config.max_discontinuities(),
                });
            }
            self.discontinuities.push(marker);
        }
        self.samples.extend_from_slice(samples);
        self.clock_samples.push(clock);
        Ok(())
    }

    /// Computes a stable digest over PCM, clocks, and discontinuity sequence.
    #[must_use]
    pub fn digest(&self) -> CaptureDigest {
        let mut digest = StableDigest::new();
        digest.update_bytes(b"aurorix-capture-v1");
        digest.update_u8(self.channels);
        digest.update_u64(u64::try_from(self.samples.len()).unwrap_or(u64::MAX));
        for sample in &self.samples {
            digest.update_u32(sample.to_bits());
        }
        digest.update_u64(u64::try_from(self.clock_samples.len()).unwrap_or(u64::MAX));
        for sample in &self.clock_samples {
            digest.update_u64(sample.clock_epoch);
            digest.update_u64(sample.rendered_frames);
            digest.update_u64(sample.media_position_frames);
            digest.update_u32(sample.output_sample_rate_hz);
            digest.update_u32(sample.estimated_output_latency_frames);
        }
        digest.update_u64(u64::try_from(self.discontinuities.len()).unwrap_or(u64::MAX));
        for marker in &self.discontinuities {
            digest.update_u64(marker.epoch);
            digest.update_u8(marker.reason.code());
        }
        CaptureDigest(digest.finish())
    }
}

/// Which fixed capture capacity was exceeded.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CaptureLimitKind {
    /// PCM sample storage.
    PcmSamples,
    /// Compact clock observations.
    ClockSamples,
    /// Discontinuity markers.
    Discontinuities,
}

/// Errors from bounded output capture.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CaptureSinkError {
    /// Interleaved sample count is not divisible by the channel count.
    MisalignedSamples {
        /// Supplied sample count.
        samples: usize,
        /// Configured channel count.
        channels: u8,
    },
    /// Zero output channels were supplied.
    InvalidChannels,
    /// A bounded count calculation overflowed.
    CapacityOverflow,
    /// A configured capacity would be exceeded.
    CapacityExceeded {
        /// The storage category.
        kind: CaptureLimitKind,
        /// The required item count.
        requested: usize,
        /// The configured maximum.
        maximum: usize,
    },
}

impl fmt::Display for CaptureSinkError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MisalignedSamples { samples, channels } => write!(
                formatter,
                "capture has {samples} samples, which is not aligned to {channels} channels"
            ),
            Self::InvalidChannels => formatter.write_str("capture channel count must be non-zero"),
            Self::CapacityOverflow => {
                formatter.write_str("capture capacity calculation overflowed")
            }
            Self::CapacityExceeded {
                kind,
                requested,
                maximum,
            } => write!(
                formatter,
                "capture {kind:?} capacity {requested} exceeds maximum {maximum}"
            ),
        }
    }
}

impl Error for CaptureSinkError {}

#[derive(Debug, Clone, Copy)]
struct StableDigest {
    lanes: [u64; 4],
}

impl StableDigest {
    const SEEDS: [u64; 4] = [
        0xcbf2_9ce4_8422_2325,
        0x9e37_79b9_7f4a_7c15,
        0x6a09_e667_f3bc_c909,
        0xbb67_ae85_84ca_a73b,
    ];

    const fn new() -> Self {
        Self { lanes: Self::SEEDS }
    }

    fn update_bytes(&mut self, bytes: &[u8]) {
        for byte in bytes {
            for (lane_index, lane) in self.lanes.iter_mut().enumerate() {
                let rotated = u64::from(*byte).wrapping_add((lane_index as u64) * 0x11);
                *lane ^= rotated;
                *lane = lane
                    .wrapping_mul(0x1000_0000_01b3)
                    .rotate_left(5 + (u32::try_from(lane_index).unwrap_or(0) * 7));
            }
        }
    }

    fn update_u8(&mut self, value: u8) {
        self.update_bytes(&[value]);
    }

    fn update_u32(&mut self, value: u32) {
        self.update_bytes(&value.to_le_bytes());
    }

    fn update_u64(&mut self, value: u64) {
        self.update_bytes(&value.to_le_bytes());
    }

    fn finish(mut self) -> [u8; 32] {
        for (index, lane) in self.lanes.iter_mut().enumerate() {
            *lane ^= (*lane >> 33).wrapping_add((index as u64) * 0x9e37_79b9);
            *lane = lane.wrapping_mul(0xff51_afd7_ed55_8ccd);
            *lane ^= *lane >> 33;
        }
        let mut output = [0; 32];
        for (index, lane) in self.lanes.into_iter().enumerate() {
            output[index * 8..(index + 1) * 8].copy_from_slice(&lane.to_be_bytes());
        }
        output
    }
}

#[cfg(test)]
mod tests {
    use super::{
        CaptureDiscontinuity, CaptureDiscontinuityReason, CaptureSink, CaptureSinkConfig,
        CaptureSinkError,
    };
    use crate::realtime::CompactClockSample;

    fn clock(epoch: u64) -> CompactClockSample {
        CompactClockSample {
            clock_epoch: epoch,
            rendered_frames: 2,
            media_position_frames: 3,
            output_sample_rate_hz: 48_000,
            estimated_output_latency_frames: 32,
        }
    }

    #[test]
    fn capture_digest_is_reproducible_for_same_observation_sequence() {
        let config = CaptureSinkConfig::new(32, 4, 2);
        let mut first = CaptureSink::new(2, config).expect("stereo capture is valid");
        let mut second = CaptureSink::new(2, config).expect("stereo capture is valid");
        for sink in [&mut first, &mut second] {
            sink.capture_observation(
                &[0.25, -0.25],
                clock(4),
                Some(CaptureDiscontinuity {
                    epoch: 4,
                    reason: CaptureDiscontinuityReason::Seek,
                }),
            )
            .expect("observation fits");
            sink.capture_observation(&[0.5, -0.5], clock(4), None)
                .expect("observation fits");
        }
        assert_eq!(first.digest(), second.digest());
        assert_eq!(first.digest().to_hex().len(), 64);
        assert_eq!(first.captured_frames(), 2);
    }

    #[test]
    fn capacity_and_alignment_are_rejected_without_partial_append() {
        let config = CaptureSinkConfig::new(2, 1, 1);
        let mut sink = CaptureSink::new(2, config).expect("stereo capture is valid");
        assert!(matches!(
            sink.capture_pcm(&[1.0]),
            Err(CaptureSinkError::MisalignedSamples { .. })
        ));
        sink.capture_pcm(&[1.0, 2.0]).expect("one frame fits");
        assert!(matches!(
            sink.capture_pcm(&[3.0, 4.0]),
            Err(CaptureSinkError::CapacityExceeded { .. })
        ));
        assert_eq!(sink.samples(), &[1.0, 2.0]);
    }

    #[test]
    fn markers_are_ordered_observations_and_channel_zero_is_invalid() {
        assert_eq!(
            CaptureSink::new(0, CaptureSinkConfig::default()),
            Err(CaptureSinkError::InvalidChannels)
        );
        let mut sink =
            CaptureSink::new(1, CaptureSinkConfig::new(4, 1, 1)).expect("mono capture is valid");
        let marker = CaptureDiscontinuity {
            epoch: 1,
            reason: CaptureDiscontinuityReason::OutputRestart,
        };
        sink.capture_discontinuity(marker).expect("marker fits");
        sink.capture_discontinuity(marker)
            .expect_err("marker capacity is full");
        assert_eq!(sink.discontinuities(), &[marker]);
    }
}
