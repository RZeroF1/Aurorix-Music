//! Worker-side FLAC decoder.

use crate::{
    decoder::{DecodeOutput, Decoder},
    errors::DecoderError,
    format::AudioFormat,
    seek::{DecoderDelayPadding, SeekMetadata, SeekTarget},
    source::RuntimeSource,
};

use super::{DecoderKind, SymphoniaState};

/// Release-one native FLAC decoder backed by Symphonia.
pub struct FlacDecoder {
    inner: SymphoniaState,
}

impl FlacDecoder {
    /// Creates a decoder that accepts only native FLAC streams.
    #[must_use]
    pub fn new() -> Self {
        Self {
            inner: SymphoniaState::new(DecoderKind::Flac),
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

impl Default for FlacDecoder {
    fn default() -> Self {
        Self::new()
    }
}

impl Decoder for FlacDecoder {
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
