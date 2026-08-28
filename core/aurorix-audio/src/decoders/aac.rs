//! Worker-side AAC-LC decoder for ADTS and ISO/MP4 containers.

use crate::{
    decoder::{DecodeOutput, Decoder},
    errors::DecoderError,
    format::AudioFormat,
    seek::{DecoderDelayPadding, SeekMetadata, SeekTarget},
    source::RuntimeSource,
};

use super::{DecoderKind, SymphoniaState};

/// Release-one AAC-LC decoder backed by Symphonia's ADTS and ISO/MP4 readers.
pub struct AacDecoder {
    inner: SymphoniaState,
}

impl AacDecoder {
    /// Creates a decoder that accepts AAC-LC in ADTS or M4A/MP4.
    #[must_use]
    pub fn new() -> Self {
        Self {
            inner: SymphoniaState::new(DecoderKind::Aac),
        }
    }

    /// Returns the validated stream format after the first decode/open.
    #[must_use]
    pub fn stream_format(&self) -> Option<AudioFormat> {
        self.inner.stream_format()
    }

    /// Returns codec delay and encoder padding discovered from the stream.
    #[must_use]
    pub fn delay_padding(&self) -> DecoderDelayPadding {
        self.inner.delay_padding()
    }
}

impl Default for AacDecoder {
    fn default() -> Self {
        Self::new()
    }
}

impl Decoder for AacDecoder {
    fn decode(
        &mut self,
        source: &mut dyn RuntimeSource,
        output: &mut [f32],
    ) -> Result<DecodeOutput, DecoderError> {
        self.inner.decode(source, output)
    }

    fn seek(
        &mut self,
        source: &mut dyn RuntimeSource,
        target: SeekTarget,
    ) -> Result<SeekMetadata, DecoderError> {
        self.inner.seek(source, target)
    }

    fn reopen(&mut self) -> Result<(), DecoderError> {
        self.inner.reopen()
    }

    fn close(&mut self) -> Result<(), DecoderError> {
        self.inner.close();
        Ok(())
    }

    fn cancel(&mut self) {
        self.inner.cancel();
    }
}
