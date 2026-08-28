//! Validated audio format values shared by decoder and output boundaries.
//!
//! The values in this module are deliberately independent of a codec or a
//! platform API.  A decoder may accept one of the bounded source formats and
//! expose the resulting stream as the ordinary interleaved `f32` PCM format
//! used by the Gate 2 data plane.

use std::{error::Error, fmt};

/// Smallest source or output sample rate accepted by the release-one matrix.
pub const MIN_SAMPLE_RATE_HZ: u32 = 8_000;

/// Largest source or output sample rate accepted by the release-one matrix.
pub const MAX_SAMPLE_RATE_HZ: u32 = 192_000;

/// Smallest supported channel count.
pub const MIN_CHANNELS: u8 = 1;

/// Largest supported channel count.
pub const MAX_CHANNELS: u8 = 8;

/// A sample representation at a format boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SampleFormat {
    /// Signed little-endian 16-bit PCM source samples.
    I16,
    /// Signed little-endian packed 24-bit PCM source samples.
    I24,
    /// Signed little-endian 32-bit PCM source samples.
    I32,
    /// The ordinary internal interleaved playback representation.
    F32,
}

impl SampleFormat {
    /// Returns the nominal number of bits represented by one sample.
    #[must_use]
    pub const fn bits(self) -> u8 {
        match self {
            Self::I16 => 16,
            Self::I24 => 24,
            Self::I32 | Self::F32 => 32,
        }
    }

    /// Returns whether this representation is one of the accepted integer
    /// source formats.
    #[must_use]
    pub const fn is_integer_source(self) -> bool {
        matches!(self, Self::I16 | Self::I24 | Self::I32)
    }

    /// Returns the source representation for a supported integer bit depth.
    ///
    /// # Errors
    ///
    /// Returns [`FormatError::UnsupportedBitDepth`] for every other value.
    pub const fn from_source_bits(bits: u8) -> Result<Self, FormatError> {
        match bits {
            16 => Ok(Self::I16),
            24 => Ok(Self::I24),
            32 => Ok(Self::I32),
            actual => Err(FormatError::UnsupportedBitDepth { actual }),
        }
    }
}

/// A validated source or intermediate audio format.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct AudioFormat {
    sample_rate_hz: u32,
    channels: u8,
    sample_format: SampleFormat,
}

impl AudioFormat {
    /// Creates a format after checking the release-one rate and channel
    /// bounds.
    ///
    /// `F32` is accepted here because it is the internal playback format;
    /// callers that model decoder input should use [`Self::source`].
    ///
    /// # Errors
    ///
    /// Returns a typed error when the rate or channel count is outside the
    /// bounded format matrix.
    pub fn new(
        sample_rate_hz: u32,
        channels: u8,
        sample_format: SampleFormat,
    ) -> Result<Self, FormatError> {
        validate_rate_and_channels(sample_rate_hz, channels)?;
        Ok(Self {
            sample_rate_hz,
            channels,
            sample_format,
        })
    }

    /// Creates a validated integer PCM source format from its bit depth.
    ///
    /// # Errors
    ///
    /// Returns a typed error when the rate, channel count, or bit depth is
    /// unsupported.
    pub fn source(sample_rate_hz: u32, channels: u8, source_bits: u8) -> Result<Self, FormatError> {
        let sample_format = SampleFormat::from_source_bits(source_bits)?;
        Self::new(sample_rate_hz, channels, sample_format)
    }

    /// Returns the validated format for interleaved `f32` PCM output.
    ///
    /// # Errors
    ///
    /// Returns a typed error when the rate or channel count is unsupported.
    pub fn f32(sample_rate_hz: u32, channels: u8) -> Result<Self, FormatError> {
        Self::new(sample_rate_hz, channels, SampleFormat::F32)
    }

    /// Returns the sample rate in hertz.
    #[must_use]
    pub const fn sample_rate_hz(self) -> u32 {
        self.sample_rate_hz
    }

    /// Returns the number of interleaved channels.
    #[must_use]
    pub const fn channels(self) -> u8 {
        self.channels
    }

    /// Returns the boundary sample representation.
    #[must_use]
    pub const fn sample_format(self) -> SampleFormat {
        self.sample_format
    }

    /// Returns whether this value describes an accepted integer source.
    #[must_use]
    pub const fn is_integer_source(self) -> bool {
        self.sample_format.is_integer_source()
    }

    /// Converts the stream dimensions into the internal `f32` PCM format.
    #[must_use]
    pub const fn to_pcm_format(self) -> PcmFormat {
        PcmFormat {
            sample_rate_hz: self.sample_rate_hz,
            channels: self.channels,
        }
    }

    /// Returns the number of interleaved samples needed for a frame count.
    ///
    /// `None` indicates arithmetic overflow.  The method does not allocate
    /// or impose a buffer-size policy; that policy belongs to `AudioBlock`.
    #[must_use]
    pub const fn sample_count_for_frames(self, frames: usize) -> Option<usize> {
        frames.checked_mul(self.channels as usize)
    }
}

/// The internal interleaved `f32` PCM format.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PcmFormat {
    sample_rate_hz: u32,
    channels: u8,
}

impl PcmFormat {
    /// Creates a validated interleaved `f32` PCM format.
    ///
    /// # Errors
    ///
    /// Returns a typed error when the rate or channel count is outside the
    /// release-one bounds.
    pub fn new(sample_rate_hz: u32, channels: u8) -> Result<Self, FormatError> {
        validate_rate_and_channels(sample_rate_hz, channels)?;
        Ok(Self {
            sample_rate_hz,
            channels,
        })
    }

    /// Returns the sample rate in hertz.
    #[must_use]
    pub const fn sample_rate_hz(self) -> u32 {
        self.sample_rate_hz
    }

    /// Returns the number of interleaved channels.
    #[must_use]
    pub const fn channels(self) -> u8 {
        self.channels
    }

    /// Returns the corresponding generic format value.
    #[must_use]
    pub const fn as_audio_format(self) -> AudioFormat {
        AudioFormat {
            sample_rate_hz: self.sample_rate_hz,
            channels: self.channels,
            sample_format: SampleFormat::F32,
        }
    }

    /// Returns the number of interleaved samples needed for a frame count.
    #[must_use]
    pub const fn sample_count_for_frames(self, frames: usize) -> Option<usize> {
        frames.checked_mul(self.channels as usize)
    }
}

/// A rejected audio format value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FormatError {
    /// The sample rate is outside the bounded matrix.
    InvalidSampleRate { actual: u32 },
    /// The channel count is outside the bounded matrix.
    InvalidChannelCount { actual: u8 },
    /// The source bit depth is not one of 16, 24, or 32.
    UnsupportedBitDepth { actual: u8 },
}

impl fmt::Display for FormatError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidSampleRate { actual } => write!(
                formatter,
                "sample rate {actual} Hz is outside {MIN_SAMPLE_RATE_HZ}..={MAX_SAMPLE_RATE_HZ} Hz"
            ),
            Self::InvalidChannelCount { actual } => write!(
                formatter,
                "channel count {actual} is outside {MIN_CHANNELS}..={MAX_CHANNELS}"
            ),
            Self::UnsupportedBitDepth { actual } => {
                write!(formatter, "source bit depth {actual} is unsupported")
            }
        }
    }
}

impl Error for FormatError {}

fn validate_rate_and_channels(sample_rate_hz: u32, channels: u8) -> Result<(), FormatError> {
    if !(MIN_SAMPLE_RATE_HZ..=MAX_SAMPLE_RATE_HZ).contains(&sample_rate_hz) {
        return Err(FormatError::InvalidSampleRate {
            actual: sample_rate_hz,
        });
    }
    if !(MIN_CHANNELS..=MAX_CHANNELS).contains(&channels) {
        return Err(FormatError::InvalidChannelCount { actual: channels });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        AudioFormat, FormatError, MAX_CHANNELS, MAX_SAMPLE_RATE_HZ, MIN_CHANNELS,
        MIN_SAMPLE_RATE_HZ, PcmFormat, SampleFormat,
    };

    #[test]
    fn source_matrix_accepts_only_the_three_integer_depths() {
        assert_eq!(
            AudioFormat::source(44_100, 2, 16).unwrap().sample_format(),
            SampleFormat::I16
        );
        assert_eq!(
            AudioFormat::source(96_000, 8, 24).unwrap().sample_format(),
            SampleFormat::I24
        );
        assert_eq!(
            AudioFormat::source(192_000, 1, 32).unwrap().sample_format(),
            SampleFormat::I32
        );
        assert_eq!(
            AudioFormat::source(44_100, 2, 20),
            Err(FormatError::UnsupportedBitDepth { actual: 20 })
        );
    }

    #[test]
    fn rate_and_channel_bounds_are_checked() {
        assert!(AudioFormat::f32(MIN_SAMPLE_RATE_HZ, MIN_CHANNELS).is_ok());
        assert!(AudioFormat::f32(MAX_SAMPLE_RATE_HZ, MAX_CHANNELS).is_ok());
        assert_eq!(
            PcmFormat::new(MIN_SAMPLE_RATE_HZ - 1, 2),
            Err(FormatError::InvalidSampleRate {
                actual: MIN_SAMPLE_RATE_HZ - 1
            })
        );
        assert_eq!(
            PcmFormat::new(MAX_SAMPLE_RATE_HZ, MAX_CHANNELS + 1),
            Err(FormatError::InvalidChannelCount {
                actual: MAX_CHANNELS + 1
            })
        );
    }

    #[test]
    fn pcm_projection_preserves_dimensions_and_counts_samples() {
        let source = AudioFormat::source(48_000, 2, 24).unwrap();
        let pcm = source.to_pcm_format();
        assert_eq!(pcm.sample_rate_hz(), 48_000);
        assert_eq!(pcm.channels(), 2);
        assert_eq!(pcm.sample_count_for_frames(128), Some(256));
        assert_eq!(source.sample_count_for_frames(128), Some(256));
    }

    #[test]
    fn sample_count_reports_overflow_without_panicking() {
        let format = PcmFormat::new(44_100, 8).unwrap();
        assert_eq!(format.sample_count_for_frames(usize::MAX), None);
    }
}
