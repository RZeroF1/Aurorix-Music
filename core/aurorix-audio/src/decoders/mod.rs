//! Release-one local-file decoders.
//!
//! The concrete decoders in this module are worker-side implementations. They
//! never run from the realtime callback and expose only bounded interleaved
//! `f32` PCM through the crate's format-neutral [`crate::decoder::Decoder`]
//! contract.

use std::io::{self, Cursor};

use symphonia::core::codecs::audio::well_known::{
    CODEC_ID_AAC, CODEC_ID_FLAC, CODEC_ID_MP3, CODEC_ID_PCM_F32BE, CODEC_ID_PCM_F32BE_PLANAR,
    CODEC_ID_PCM_F32LE, CODEC_ID_PCM_F32LE_PLANAR, CODEC_ID_PCM_S16BE, CODEC_ID_PCM_S16BE_PLANAR,
    CODEC_ID_PCM_S16LE, CODEC_ID_PCM_S16LE_PLANAR, CODEC_ID_PCM_S24BE, CODEC_ID_PCM_S24BE_PLANAR,
    CODEC_ID_PCM_S24LE, CODEC_ID_PCM_S24LE_PLANAR, CODEC_ID_PCM_S32BE, CODEC_ID_PCM_S32BE_PLANAR,
    CODEC_ID_PCM_S32LE, CODEC_ID_PCM_S32LE_PLANAR, CODEC_ID_PCM_U16BE, CODEC_ID_PCM_U16BE_PLANAR,
    CODEC_ID_PCM_U16LE, CODEC_ID_PCM_U16LE_PLANAR, CODEC_ID_PCM_U24BE, CODEC_ID_PCM_U24BE_PLANAR,
    CODEC_ID_PCM_U24LE, CODEC_ID_PCM_U24LE_PLANAR, CODEC_ID_PCM_U32BE, CODEC_ID_PCM_U32BE_PLANAR,
    CODEC_ID_PCM_U32LE, CODEC_ID_PCM_U32LE_PLANAR,
};
use symphonia::core::{
    codecs::{CodecParameters, audio::AudioDecoderOptions},
    errors::Error as SymphoniaError,
    formats::{FormatOptions, FormatReader, SeekMode, SeekTo, TrackType, probe::Hint},
    io::{MediaSourceStream, MediaSourceStreamOptions},
    meta::MetadataOptions,
    units::Time,
};

use crate::{
    decoder::{DecodeOutput, Decoder},
    errors::{DecoderError, DecoderFailureStage},
    format::{AudioFormat, FormatError},
    seek::{DecoderDelayPadding, SeekMetadata, SeekTarget},
    source::RuntimeSource,
};

mod aac;
mod flac;
mod mp3;
mod opus;
mod wav;

pub use aac::AacDecoder;
pub use flac::FlacDecoder;
pub use mp3::Mp3Decoder;
pub use opus::OpusDecoder;
pub use wav::WavDecoder;

/// The release-one decoder selected by a known file extension.
#[must_use]
pub fn decoder_for_extension(extension: &str) -> Option<Box<dyn Decoder>> {
    match extension
        .trim_start_matches('.')
        .to_ascii_lowercase()
        .as_str()
    {
        "wav" | "wave" => Some(Box::new(WavDecoder::new())),
        "flac" => Some(Box::new(FlacDecoder::new())),
        "mp3" => Some(Box::new(Mp3Decoder::new())),
        "aac" | "adts" | "m4a" | "mp4" => Some(Box::new(AacDecoder::new())),
        "ogg" | "opus" => Some(Box::new(OpusDecoder::new())),
        _ => None,
    }
}

/// Returns the extensions covered by [`decoder_for_extension`].
#[must_use]
pub const fn supported_extensions() -> &'static [&'static str] {
    &[
        "wav", "wave", "flac", "mp3", "aac", "adts", "m4a", "mp4", "ogg", "opus",
    ]
}

/// Maximum encoded source materialized by one worker-side decoder.
///
/// The existing source contract bounds each read to 64 KiB. Materializing a
/// bounded local source lets Symphonia own a seekable reader without storing a
/// borrow of the worker's runtime source. This is a worker memory policy, not
/// a realtime allocation.
pub(crate) const MAX_MATERIALIZED_SOURCE_BYTES: u64 = 512 * 1024 * 1024;
pub(crate) const SOURCE_READ_CHUNK_BYTES: usize = 64 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DecoderKind {
    Wav,
    Flac,
    Mp3,
    Aac,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ContainerKind {
    Wav,
    Flac,
    Mp3,
    Adts,
}

/// Shared Symphonia state used by WAV, FLAC, MP3, and AAC wrappers.
pub(crate) struct SymphoniaState {
    kind: DecoderKind,
    bytes: Option<Vec<u8>>,
    format: Option<Box<dyn FormatReader + 'static>>,
    decoder: Option<Box<dyn symphonia::core::codecs::audio::AudioDecoder>>,
    track_id: Option<u32>,
    stream_format: Option<AudioFormat>,
    delay_padding: DecoderDelayPadding,
    pending: Vec<f32>,
    pending_offset: usize,
    discard_frames_after_seek: u64,
    eof: bool,
    cancelled: bool,
    closed: bool,
}

impl SymphoniaState {
    pub(crate) fn new(kind: DecoderKind) -> Self {
        Self {
            kind,
            bytes: None,
            format: None,
            decoder: None,
            track_id: None,
            stream_format: None,
            delay_padding: DecoderDelayPadding::default(),
            pending: Vec::new(),
            pending_offset: 0,
            discard_frames_after_seek: 0,
            eof: false,
            cancelled: false,
            closed: false,
        }
    }

    pub(crate) fn stream_format(&self) -> Option<AudioFormat> {
        self.stream_format
    }

    pub(crate) fn delay_padding(&self) -> DecoderDelayPadding {
        self.delay_padding
    }

    pub(crate) fn decode(
        &mut self,
        source: &mut dyn RuntimeSource,
        output: &mut [f32],
    ) -> Result<DecodeOutput, DecoderError> {
        self.ensure_available()?;
        if output.is_empty() {
            return Ok(DecodeOutput::with_delay_padding(
                0,
                self.eof,
                self.delay_padding,
            ));
        }
        self.ensure_open(source)?;

        let channels = usize::from(
            self.stream_format
                .ok_or_else(|| {
                    DecoderError::failure(DecoderFailureStage::Decode, "missing stream format")
                })?
                .channels(),
        );
        if !output.len().is_multiple_of(channels) {
            return Err(DecoderError::InvalidOutput {
                samples_written: output.len(),
                capacity: output.len() - (output.len() % channels),
            });
        }

        let mut written = self.copy_pending(output);
        while written < output.len() && !self.eof {
            self.fill_pending()?;
            written += self.copy_pending(&mut output[written..]);
        }
        Ok(DecodeOutput::with_delay_padding(
            written,
            self.eof && self.pending_remaining() == 0,
            self.delay_padding,
        ))
    }

    pub(crate) fn seek(
        &mut self,
        source: &mut dyn RuntimeSource,
        target: SeekTarget,
    ) -> Result<SeekMetadata, DecoderError> {
        self.ensure_available()?;
        self.ensure_open(source)?;
        let stream_format = self.stream_format.ok_or_else(|| {
            DecoderError::failure(DecoderFailureStage::Seek, "missing stream format")
        })?;
        let track_id = self.track_id.ok_or_else(|| {
            DecoderError::failure(DecoderFailureStage::Seek, "missing audio track")
        })?;
        let target_frame = match target {
            SeekTarget::Microseconds(value) => value
                .checked_mul(u64::from(stream_format.sample_rate_hz()))
                .and_then(|value| value.checked_div(1_000_000))
                .ok_or_else(|| {
                    DecoderError::failure(DecoderFailureStage::Seek, "microsecond seek overflow")
                })?,
            SeekTarget::Frame(frame) => frame,
            SeekTarget::ByteOffset(_) => {
                return Err(DecoderError::unsupported(
                    "container byte-offset seek is not a media seek target",
                ));
            }
        };
        let time = match target {
            SeekTarget::Microseconds(value) => Time::from_micros_u64(value),
            SeekTarget::Frame(frame) => {
                let micros = frame
                    .checked_mul(1_000_000)
                    .and_then(|value| value.checked_div(u64::from(stream_format.sample_rate_hz())))
                    .ok_or_else(|| {
                        DecoderError::failure(DecoderFailureStage::Seek, "frame seek overflow")
                    })?;
                Time::from_micros_u64(micros)
            }
            SeekTarget::ByteOffset(_) => unreachable!("byte-offset seek was rejected above"),
        };
        let format = self.format.as_mut().ok_or_else(|| {
            DecoderError::failure(DecoderFailureStage::Seek, "missing format reader")
        })?;
        let seeked = format
            .seek(
                SeekMode::Accurate,
                SeekTo::Time {
                    time,
                    track_id: Some(track_id),
                },
            )
            .map_err(|error| map_symphonia_error(DecoderFailureStage::Seek, error))?;
        if let Some(decoder) = self.decoder.as_mut() {
            decoder.reset();
        }
        self.pending.clear();
        self.pending_offset = 0;
        let actual_frame = u64::try_from(seeked.actual_ts.get()).unwrap_or(0);
        self.discard_frames_after_seek = target_frame.saturating_sub(actual_frame);
        self.eof = false;
        Ok(SeekMetadata::new(self.delay_padding, None))
    }

    pub(crate) fn reopen(&mut self) -> Result<(), DecoderError> {
        self.ensure_available()?;
        self.bytes = None;
        self.format = None;
        self.decoder = None;
        self.track_id = None;
        self.stream_format = None;
        self.delay_padding = DecoderDelayPadding::default();
        self.pending.clear();
        self.pending_offset = 0;
        self.discard_frames_after_seek = 0;
        self.eof = false;
        Ok(())
    }

    pub(crate) fn close(&mut self) {
        self.bytes = None;
        self.format = None;
        self.decoder = None;
        self.pending.clear();
        self.pending_offset = 0;
        self.discard_frames_after_seek = 0;
        self.closed = true;
    }

    pub(crate) fn cancel(&mut self) {
        self.cancelled = true;
        self.bytes = None;
        self.format = None;
        self.decoder = None;
        self.pending.clear();
        self.pending_offset = 0;
        self.discard_frames_after_seek = 0;
    }

    fn ensure_available(&self) -> Result<(), DecoderError> {
        if self.cancelled {
            return Err(DecoderError::Cancelled);
        }
        if self.closed {
            return Err(DecoderError::Closed);
        }
        Ok(())
    }

    fn ensure_open(&mut self, source: &mut dyn RuntimeSource) -> Result<(), DecoderError> {
        if self.format.is_some() {
            return Ok(());
        }
        let bytes = match self.bytes.take() {
            Some(bytes) => bytes,
            None => read_source(source)?,
        };
        let container = expected_container(self.kind);
        let mut hint = Hint::new();
        if self.kind == DecoderKind::Aac {
            hint.with_extension("m4a");
        } else {
            hint.with_extension(container.extension());
        }
        let mss = MediaSourceStream::new(
            Box::new(Cursor::new(bytes.clone())),
            MediaSourceStreamOptions::default(),
        );
        let format = symphonia::default::get_probe()
            .probe(
                &hint,
                mss,
                FormatOptions::default(),
                MetadataOptions::default(),
            )
            .map_err(|error| map_symphonia_error(DecoderFailureStage::Open, error))?;
        let track = format.default_track(TrackType::Audio).ok_or_else(|| {
            DecoderError::corrupt("container does not contain a default audio track")
        })?;
        let params = track
            .codec_params
            .as_ref()
            .ok_or_else(|| DecoderError::corrupt("audio track has no codec parameters"))?;
        validate_track(self.kind, format.format_info().short_name, params)?;
        if self.kind == DecoderKind::Aac && format.format_info().short_name == "aac" {
            validate_adts_aac_lc(&bytes)?;
        }
        let sample_rate = params
            .audio()
            .and_then(|audio| audio.sample_rate)
            .ok_or_else(|| DecoderError::corrupt("audio track has no sample rate"))?;
        let channels = params
            .audio()
            .and_then(|audio| audio.channels.as_ref())
            .map(symphonia::core::audio::Channels::count)
            .ok_or_else(|| DecoderError::corrupt("audio track has no channel layout"))?;
        let stream_format = AudioFormat::f32(sample_rate, u8::try_from(channels).unwrap_or(0))
            .map_err(map_format_error)?;
        let delay_padding = DecoderDelayPadding::new(
            u64::from(track.delay.unwrap_or(0)),
            u64::from(track.padding.unwrap_or(0)),
        );
        let decoder = symphonia::default::get_codecs()
            .make_audio_decoder(
                params
                    .audio()
                    .ok_or_else(|| DecoderError::corrupt("selected track is not an audio codec"))?,
                &AudioDecoderOptions::default(),
            )
            .map_err(|error| map_symphonia_error(DecoderFailureStage::Open, error))?;
        self.bytes = Some(bytes);
        self.track_id = Some(track.id);
        self.stream_format = Some(stream_format);
        self.delay_padding = delay_padding;
        self.decoder = Some(decoder);
        self.format = Some(format);
        Ok(())
    }

    fn fill_pending(&mut self) -> Result<(), DecoderError> {
        if self.pending_remaining() != 0 || self.eof {
            return Ok(());
        }
        let track_id = self.track_id.ok_or_else(|| {
            DecoderError::failure(DecoderFailureStage::Decode, "missing audio track")
        })?;
        let channels = usize::from(
            self.stream_format
                .ok_or_else(|| {
                    DecoderError::failure(DecoderFailureStage::Decode, "missing stream format")
                })?
                .channels(),
        );
        loop {
            let packet = self
                .format
                .as_mut()
                .ok_or_else(|| {
                    DecoderError::failure(DecoderFailureStage::Decode, "missing format reader")
                })?
                .next_packet()
                .map_err(|error| map_symphonia_error(DecoderFailureStage::Read, error))?;
            let Some(packet) = packet else {
                self.eof = true;
                return Ok(());
            };
            if packet.track_id != track_id {
                continue;
            }
            let codec_decoder = self.decoder.as_mut().ok_or_else(|| {
                DecoderError::failure(DecoderFailureStage::Decode, "missing codec decoder")
            })?;
            let decoded = codec_decoder
                .decode(&packet)
                .map_err(|error| map_symphonia_error(DecoderFailureStage::Decode, error))?;
            if decoded.is_empty() {
                continue;
            }
            let decoded_samples = decoded.samples_interleaved();
            if !decoded_samples.is_multiple_of(channels) {
                return Err(DecoderError::corrupt(
                    "decoder returned samples misaligned to the stream channels",
                ));
            }
            self.pending.resize(decoded_samples, 0.0);
            decoded.copy_to_slice_interleaved(&mut self.pending);
            if self.discard_frames_after_seek != 0 {
                let decoded_frames = decoded_samples / channels;
                let discard_frames = self
                    .discard_frames_after_seek
                    .min(u64::try_from(decoded_frames).unwrap_or(u64::MAX));
                let discard_samples = usize::try_from(discard_frames)
                    .ok()
                    .and_then(|frames| frames.checked_mul(channels))
                    .ok_or_else(|| {
                        DecoderError::failure(
                            DecoderFailureStage::Decode,
                            "seek discard count overflow",
                        )
                    })?;
                self.discard_frames_after_seek -= discard_frames;
                if discard_samples != 0 {
                    let remaining_samples = decoded_samples - discard_samples;
                    self.pending
                        .copy_within(discard_samples..decoded_samples, 0);
                    self.pending.truncate(remaining_samples);
                }
            }
            self.pending_offset = 0;
            if self.pending.is_empty() {
                continue;
            }
            return Ok(());
        }
    }

    fn copy_pending(&mut self, output: &mut [f32]) -> usize {
        let remaining = self.pending_remaining();
        let count = remaining.min(output.len());
        if count == 0 {
            return 0;
        }
        output[..count]
            .copy_from_slice(&self.pending[self.pending_offset..self.pending_offset + count]);
        self.pending_offset += count;
        if self.pending_offset == self.pending.len() {
            self.pending.clear();
            self.pending_offset = 0;
        }
        count
    }

    fn pending_remaining(&self) -> usize {
        self.pending.len().saturating_sub(self.pending_offset)
    }
}

impl Decoder for SymphoniaState {
    fn decode(
        &mut self,
        source: &mut dyn RuntimeSource,
        output: &mut [f32],
    ) -> Result<DecodeOutput, DecoderError> {
        Self::decode(self, source, output)
    }

    fn seek(
        &mut self,
        source: &mut dyn RuntimeSource,
        target: SeekTarget,
    ) -> Result<SeekMetadata, DecoderError> {
        Self::seek(self, source, target)
    }

    fn reopen(&mut self) -> Result<(), DecoderError> {
        Self::reopen(self)
    }

    fn close(&mut self) -> Result<(), DecoderError> {
        Self::close(self);
        Ok(())
    }

    fn cancel(&mut self) {
        Self::cancel(self);
    }
}

fn expected_container(kind: DecoderKind) -> ContainerKind {
    match kind {
        DecoderKind::Wav => ContainerKind::Wav,
        DecoderKind::Flac => ContainerKind::Flac,
        DecoderKind::Mp3 => ContainerKind::Mp3,
        DecoderKind::Aac => ContainerKind::Adts,
    }
}

pub(crate) fn map_opus_error<E: std::fmt::Debug>(error: E) -> DecoderError {
    DecoderError::corrupt(format!("Opus packet rejected: {error:?}"))
}

fn validate_track(
    kind: DecoderKind,
    container_name: &str,
    params: &CodecParameters,
) -> Result<(), DecoderError> {
    let audio = params
        .audio()
        .ok_or_else(|| DecoderError::corrupt("selected track is not audio"))?;
    let actual_container = match kind {
        DecoderKind::Wav => matches!(container_name, "wave" | "wav"),
        DecoderKind::Flac => container_name == "flac",
        DecoderKind::Mp3 => container_name == "mp3",
        DecoderKind::Aac => matches!(container_name, "aac" | "isomp4"),
    };
    if !actual_container {
        return Err(DecoderError::unsupported(
            "selected decoder/container mismatch",
        ));
    }
    let codec_supported = match kind {
        DecoderKind::Wav => matches!(
            audio.codec,
            CODEC_ID_PCM_F32BE
                | CODEC_ID_PCM_F32LE
                | CODEC_ID_PCM_F32BE_PLANAR
                | CODEC_ID_PCM_F32LE_PLANAR
                | CODEC_ID_PCM_S16BE
                | CODEC_ID_PCM_S16BE_PLANAR
                | CODEC_ID_PCM_S16LE
                | CODEC_ID_PCM_S16LE_PLANAR
                | CODEC_ID_PCM_S24BE
                | CODEC_ID_PCM_S24BE_PLANAR
                | CODEC_ID_PCM_S24LE
                | CODEC_ID_PCM_S24LE_PLANAR
                | CODEC_ID_PCM_S32BE
                | CODEC_ID_PCM_S32BE_PLANAR
                | CODEC_ID_PCM_S32LE
                | CODEC_ID_PCM_S32LE_PLANAR
                | CODEC_ID_PCM_U16BE
                | CODEC_ID_PCM_U16BE_PLANAR
                | CODEC_ID_PCM_U16LE
                | CODEC_ID_PCM_U16LE_PLANAR
                | CODEC_ID_PCM_U24BE
                | CODEC_ID_PCM_U24BE_PLANAR
                | CODEC_ID_PCM_U24LE
                | CODEC_ID_PCM_U24LE_PLANAR
                | CODEC_ID_PCM_U32BE
                | CODEC_ID_PCM_U32BE_PLANAR
                | CODEC_ID_PCM_U32LE
                | CODEC_ID_PCM_U32LE_PLANAR
        ),
        DecoderKind::Flac => audio.codec == CODEC_ID_FLAC,
        DecoderKind::Mp3 => audio.codec == CODEC_ID_MP3,
        DecoderKind::Aac => audio.codec == CODEC_ID_AAC,
    };
    if !codec_supported {
        return Err(DecoderError::unsupported(
            "codec is outside this decoder's release-one scope",
        ));
    }
    if kind == DecoderKind::Aac
        && let Some(profile) = audio.profile
        && profile != symphonia::core::codecs::audio::well_known::profiles::CODEC_PROFILE_AAC_LC
    {
        return Err(DecoderError::unsupported("AAC profile is not AAC-LC"));
    }
    if matches!(kind, DecoderKind::Wav | DecoderKind::Flac)
        && let Some(bits) = audio.bits_per_coded_sample.or(audio.bits_per_sample)
        && !matches!(bits, 16 | 24 | 32)
    {
        return Err(DecoderError::unsupported(
            "source bit depth is outside 16/24/32-bit bounds",
        ));
    }
    Ok(())
}

fn validate_adts_aac_lc(bytes: &[u8]) -> Result<(), DecoderError> {
    let mut position = 0_usize;
    let mut frame_count = 0_usize;
    while position < bytes.len() {
        let remaining = bytes.len() - position;
        if remaining < 7 {
            return Err(DecoderError::corrupt("truncated ADTS header"));
        }
        let header = &bytes[position..];
        if header[0] != 0xff || header[1] & 0xf6 != 0xf0 {
            return Err(DecoderError::corrupt("invalid ADTS sync or layer"));
        }
        let profile = ((header[2] >> 6) & 0x03) + 1;
        if profile != 2 {
            return Err(DecoderError::unsupported("AAC profile is not AAC-LC"));
        }
        let header_length = if header[1] & 1 == 0 { 9 } else { 7 };
        if remaining < header_length {
            return Err(DecoderError::corrupt("truncated ADTS CRC header"));
        }
        let frame_length = ((usize::from(header[3]) & 0x03) << 11)
            | (usize::from(header[4]) << 3)
            | (usize::from(header[5]) >> 5);
        if frame_length < header_length {
            return Err(DecoderError::corrupt(
                "ADTS frame is shorter than its header",
            ));
        }
        let end = position
            .checked_add(frame_length)
            .ok_or_else(|| DecoderError::corrupt("ADTS frame length overflow"))?;
        if end > bytes.len() {
            return Err(DecoderError::corrupt("truncated ADTS frame"));
        }
        if header[6] & 0x03 != 0 {
            return Err(DecoderError::unsupported(
                "ADTS multiple raw data blocks are not supported",
            ));
        }
        position = end;
        frame_count += 1;
    }
    if frame_count == 0 {
        return Err(DecoderError::corrupt("ADTS stream has no frames"));
    }
    Ok(())
}

pub(crate) fn read_source(source: &mut dyn RuntimeSource) -> Result<Vec<u8>, DecoderError> {
    if let Some(length) = source.len()
        && length > MAX_MATERIALIZED_SOURCE_BYTES
    {
        return Err(DecoderError::Unavailable {
            reason: crate::errors::UnavailableReason::TooLarge,
        });
    }
    let mut bytes = Vec::new();
    if let Some(length) = source.len() {
        let capacity = usize::try_from(length.min(MAX_MATERIALIZED_SOURCE_BYTES)).unwrap_or(0);
        bytes.reserve(capacity);
    }
    let mut chunk = vec![0_u8; SOURCE_READ_CHUNK_BYTES];
    loop {
        let read = source
            .read_bounded(&mut chunk)
            .map_err(DecoderError::from)?;
        if read.bytes_read() > chunk.len() {
            return Err(DecoderError::corrupt(
                "source returned an invalid read length",
            ));
        }
        bytes.extend_from_slice(&chunk[..read.bytes_read()]);
        if u64::try_from(bytes.len()).unwrap_or(u64::MAX) > MAX_MATERIALIZED_SOURCE_BYTES {
            return Err(DecoderError::Unavailable {
                reason: crate::errors::UnavailableReason::TooLarge,
            });
        }
        if read.end_of_stream() {
            break;
        }
        if read.bytes_read() == 0 {
            return Err(DecoderError::corrupt("source made no progress before EOF"));
        }
    }
    if bytes.is_empty() {
        return Err(DecoderError::corrupt("audio source is empty"));
    }
    Ok(bytes)
}

impl ContainerKind {
    const fn extension(self) -> &'static str {
        match self {
            Self::Wav => "wav",
            Self::Flac => "flac",
            Self::Mp3 => "mp3",
            Self::Adts => "aac",
        }
    }
}

fn map_format_error(error: FormatError) -> DecoderError {
    DecoderError::unsupported(error.to_string())
}

fn map_symphonia_error(stage: DecoderFailureStage, error: SymphoniaError) -> DecoderError {
    match error {
        SymphoniaError::Unsupported(_) => {
            DecoderError::unsupported("unsupported container or codec feature")
        }
        SymphoniaError::DecodeError(_) | SymphoniaError::ResetRequired => {
            DecoderError::corrupt("container or codec data is malformed")
        }
        SymphoniaError::LimitError(_) => DecoderError::Unavailable {
            reason: crate::errors::UnavailableReason::TooLarge,
        },
        SymphoniaError::SeekError(_) => DecoderError::failure(stage, "container seek failed"),
        SymphoniaError::IoError(error) => {
            DecoderError::failure(stage, io_kind_detail(error.kind()))
        }
        _ => DecoderError::failure(stage, "decoder operation failed"),
    }
}

fn io_kind_detail(kind: io::ErrorKind) -> String {
    format!("I/O error: {kind:?}")
}

#[cfg(test)]
mod tests {
    use std::io::SeekFrom;

    use super::supported_extensions;
    use crate::{
        errors::SourceError,
        seek::SeekTarget,
        source::{RuntimeSource, SourceRead},
    };

    struct MemorySource {
        bytes: Vec<u8>,
        position: usize,
    }

    impl MemorySource {
        fn new(bytes: Vec<u8>) -> Self {
            Self { bytes, position: 0 }
        }
    }

    impl RuntimeSource for MemorySource {
        fn read_bounded(&mut self, destination: &mut [u8]) -> Result<SourceRead, SourceError> {
            if destination.len() > 64 * 1024 {
                return Err(SourceError::ReadLimitExceeded {
                    requested: destination.len(),
                    maximum: 64 * 1024,
                });
            }
            let remaining = self.bytes.len().saturating_sub(self.position);
            let count = remaining.min(destination.len());
            destination[..count].copy_from_slice(&self.bytes[self.position..self.position + count]);
            self.position += count;
            Ok(SourceRead::new(count, self.position == self.bytes.len()))
        }

        fn seek(&mut self, position: SeekFrom) -> Result<u64, SourceError> {
            let target = match position {
                SeekFrom::Start(value) => value,
                SeekFrom::Current(value) => {
                    let current = i128::try_from(self.position).unwrap_or(i128::MAX);
                    let target = current + i128::from(value);
                    u64::try_from(target).map_err(|_| SourceError::SeekOutOfBounds {
                        position: 0,
                        length: u64::try_from(self.bytes.len()).expect("fixture length fits u64"),
                    })?
                }
                SeekFrom::End(value) => {
                    let end = i128::try_from(self.bytes.len()).unwrap_or(i128::MAX);
                    let target = end + i128::from(value);
                    u64::try_from(target).map_err(|_| SourceError::SeekOutOfBounds {
                        position: 0,
                        length: u64::try_from(self.bytes.len()).expect("fixture length fits u64"),
                    })?
                }
            };
            let target = usize::try_from(target).map_err(|_| SourceError::SeekOutOfBounds {
                position: target,
                length: u64::try_from(self.bytes.len()).expect("fixture length fits u64"),
            })?;
            if target > self.bytes.len() {
                return Err(SourceError::SeekOutOfBounds {
                    position: u64::try_from(target).expect("fixture position fits u64"),
                    length: u64::try_from(self.bytes.len()).expect("fixture length fits u64"),
                });
            }
            self.position = target;
            Ok(u64::try_from(target).expect("fixture position fits u64"))
        }

        fn reopen(&mut self) -> Result<(), SourceError> {
            self.position = 0;
            Ok(())
        }

        fn position(&mut self) -> Result<u64, SourceError> {
            Ok(self.position as u64)
        }

        fn len(&self) -> Option<u64> {
            Some(self.bytes.len() as u64)
        }

        fn close(&mut self) -> Result<(), SourceError> {
            Ok(())
        }

        fn cancel(&mut self) {}

        fn is_cancelled(&self) -> bool {
            false
        }
    }

    fn pcm_wav() -> Vec<u8> {
        let samples = [0_i16, 1_000, -1_000, 2_000];
        let data_size = u32::try_from(samples.len() * 2).expect("fixture fits u32");
        let riff_size = 36 + data_size;
        let mut bytes =
            Vec::with_capacity(44 + usize::try_from(data_size).expect("fixture size fits usize"));
        bytes.extend_from_slice(b"RIFF");
        bytes.extend_from_slice(&riff_size.to_le_bytes());
        bytes.extend_from_slice(b"WAVEfmt ");
        bytes.extend_from_slice(&16_u32.to_le_bytes());
        bytes.extend_from_slice(&1_u16.to_le_bytes());
        bytes.extend_from_slice(&1_u16.to_le_bytes());
        bytes.extend_from_slice(&48_000_u32.to_le_bytes());
        bytes.extend_from_slice(&96_000_u32.to_le_bytes());
        bytes.extend_from_slice(&2_u16.to_le_bytes());
        bytes.extend_from_slice(&16_u16.to_le_bytes());
        bytes.extend_from_slice(b"data");
        bytes.extend_from_slice(&data_size.to_le_bytes());
        for sample in samples {
            bytes.extend_from_slice(&sample.to_le_bytes());
        }
        bytes
    }

    #[test]
    fn release_one_extensions_are_explicit() {
        assert_eq!(supported_extensions().len(), 10);
    }

    #[test]
    fn wav_decoder_decodes_pcm_and_seeks_by_frame() {
        let mut source = MemorySource::new(pcm_wav());
        let mut decoder = super::decoder_for_extension(".WAV").expect("WAV decoder exists");
        let mut output = [0.0_f32; 4];

        let first_output = decoder
            .decode(&mut source, &mut output)
            .expect("WAV decode succeeds");
        assert_eq!(first_output.samples_written(), 4);
        assert!(output[0].abs() < f32::EPSILON);
        assert!((output[1] - (1_000.0 / 32_768.0)).abs() < 0.0001);
        let mut end_output = [0.0_f32; 4];
        let end = decoder
            .decode(&mut source, &mut end_output)
            .expect("WAV EOF probe succeeds");
        assert_eq!(end.samples_written(), 0);
        assert!(end.end_of_stream());

        let _ = decoder
            .seek(&mut source, SeekTarget::Frame(2))
            .expect("WAV frame seek succeeds");
        let seek_output = decoder
            .decode(&mut source, &mut output)
            .expect("WAV decode after seek succeeds");
        assert_eq!(seek_output.samples_written(), 2);
        assert!((output[0] - (-1_000.0 / 32_768.0)).abs() < 0.0001);
    }
}
