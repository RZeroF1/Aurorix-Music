//! Worker-side Ogg Opus decoder.
//!
//! Ogg packetization is handled by the pure Rust `ogg` crate and Opus packet
//! decoding by the pure Rust `opus-decoder` crate. The realtime boundary only
//! sees the bounded interleaved `f32` output of this worker.

use std::io::{Cursor, SeekFrom};

use ogg::{OggReadError, PacketReader};
use opus_decoder::OpusMultistreamDecoder;

use crate::{
    decoder::{DecodeOutput, Decoder},
    errors::{DecoderError, DecoderFailureStage},
    format::AudioFormat,
    seek::{DecoderDelayPadding, SeekMetadata, SeekTarget},
    source::RuntimeSource,
};

use super::{map_opus_error, read_source};

const OPUS_OUTPUT_RATE_HZ: u32 = 48_000;
const OPUS_MAX_PACKET_FRAMES: usize = 5_760;
const OPUS_MAX_CHANNELS: usize = 8;

/// Release-one Ogg Opus decoder.
pub struct OpusDecoder {
    bytes: Option<Vec<u8>>,
    reader: Option<PacketReader<Cursor<Vec<u8>>>>,
    decoder: Option<OpusMultistreamDecoder>,
    stream_format: Option<AudioFormat>,
    channels: usize,
    pre_skip_remaining: usize,
    delay_padding: DecoderDelayPadding,
    emitted_frames: u64,
    pending: Vec<f32>,
    pending_offset: usize,
    eof: bool,
    cancelled: bool,
    closed: bool,
}

impl OpusDecoder {
    /// Creates an unopened Ogg Opus decoder.
    #[must_use]
    pub fn new() -> Self {
        Self {
            bytes: None,
            reader: None,
            decoder: None,
            stream_format: None,
            channels: 0,
            pre_skip_remaining: 0,
            delay_padding: DecoderDelayPadding::default(),
            emitted_frames: 0,
            pending: Vec::new(),
            pending_offset: 0,
            eof: false,
            cancelled: false,
            closed: false,
        }
    }

    /// Returns the validated 48 kHz stream format after the first decode/open.
    #[must_use]
    pub fn stream_format(&self) -> Option<AudioFormat> {
        self.stream_format
    }

    /// Returns Opus pre-skip and end-padding metadata.
    #[must_use]
    pub fn delay_padding(&self) -> DecoderDelayPadding {
        self.delay_padding
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
        if self.reader.is_some() {
            return Ok(());
        }
        let bytes = read_source(source)?;
        self.bytes = Some(bytes.clone());
        self.reader = Some(PacketReader::new(Cursor::new(bytes)));
        self.read_headers()
    }

    fn read_headers(&mut self) -> Result<(), DecoderError> {
        let head = self.next_packet("OpusHead")?;
        let setup = parse_opus_head(&head.data)?;
        let tags = self.next_packet("OpusTags")?;
        if !tags.data.starts_with(b"OpusTags") {
            return Err(DecoderError::corrupt("Ogg Opus stream is missing OpusTags"));
        }

        let decoder = OpusMultistreamDecoder::new(
            OPUS_OUTPUT_RATE_HZ,
            setup.channels,
            setup.streams,
            setup.coupled_streams,
            &setup.mapping,
        )
        .map_err(map_opus_error)?;
        self.stream_format = Some(
            AudioFormat::f32(
                OPUS_OUTPUT_RATE_HZ,
                u8::try_from(setup.channels).unwrap_or(0),
            )
            .map_err(|error| DecoderError::unsupported(error.to_string()))?,
        );
        self.channels = setup.channels;
        self.pre_skip_remaining = setup.pre_skip;
        self.delay_padding = DecoderDelayPadding::new(setup.pre_skip as u64, 0);
        self.decoder = Some(decoder);
        self.emitted_frames = 0;
        self.pending.clear();
        self.pending_offset = 0;
        self.eof = false;
        Ok(())
    }

    fn next_packet(&mut self, expected: &'static str) -> Result<ogg::Packet, DecoderError> {
        let packet = self
            .reader
            .as_mut()
            .ok_or_else(|| DecoderError::failure(DecoderFailureStage::Open, "missing Ogg reader"))?
            .read_packet()
            .map_err(|error| map_ogg_error(&error))?
            .ok_or_else(|| DecoderError::corrupt(format!("Ogg stream ended before {expected}")))?;
        if expected == "OpusHead" && !packet.data.starts_with(b"OpusHead") {
            return Err(DecoderError::unsupported("Ogg stream is not Opus"));
        }
        Ok(packet)
    }

    fn fill_pending(&mut self) -> Result<(), DecoderError> {
        if self.pending_remaining() != 0 || self.eof {
            return Ok(());
        }
        let packet = self
            .reader
            .as_mut()
            .ok_or_else(|| {
                DecoderError::failure(DecoderFailureStage::Decode, "missing Ogg reader")
            })?
            .read_packet()
            .map_err(|error| map_ogg_error(&error))?;
        let Some(packet) = packet else {
            self.eof = true;
            return Ok(());
        };
        if packet.data.starts_with(b"OpusHead") || packet.data.starts_with(b"OpusTags") {
            return Err(DecoderError::corrupt(
                "duplicate Ogg Opus identification headers",
            ));
        }
        let mut decoded = vec![0.0_f32; OPUS_MAX_PACKET_FRAMES * self.channels];
        let frames = self
            .decoder
            .as_mut()
            .ok_or_else(|| {
                DecoderError::failure(DecoderFailureStage::Decode, "missing Opus decoder")
            })?
            .decode_float(&packet.data, &mut decoded, false)
            .map_err(map_opus_error)?;

        let mut start_frame = 0_usize;
        if self.pre_skip_remaining != 0 {
            let skipped = self.pre_skip_remaining.min(frames);
            self.pre_skip_remaining -= skipped;
            start_frame = skipped;
        }
        let keep_frames =
            self.final_packet_frame_limit(&packet, frames, start_frame, self.emitted_frames);
        if packet.last_in_stream() {
            self.eof = true;
        }
        if keep_frames == 0 {
            return Ok(());
        }
        let start_sample = start_frame * self.channels;
        let end_sample = start_sample + keep_frames * self.channels;
        self.pending
            .extend_from_slice(&decoded[start_sample..end_sample]);
        Ok(())
    }

    fn copy_pending(&mut self, output: &mut [f32]) -> usize {
        let count = self.pending_remaining().min(output.len());
        if count == 0 {
            return 0;
        }
        output[..count]
            .copy_from_slice(&self.pending[self.pending_offset..self.pending_offset + count]);
        self.pending_offset += count;
        self.emitted_frames = self
            .emitted_frames
            .saturating_add(u64::try_from(count / self.channels).unwrap_or(0));
        if self.pending_offset == self.pending.len() {
            self.pending.clear();
            self.pending_offset = 0;
        }
        count
    }

    fn pending_remaining(&self) -> usize {
        self.pending.len().saturating_sub(self.pending_offset)
    }

    fn final_packet_frame_limit(
        &self,
        packet: &ogg::Packet,
        decoded_frames: usize,
        start_frame: usize,
        already_emitted: u64,
    ) -> usize {
        let playable = decoded_frames.saturating_sub(start_frame);
        if !packet.last_in_stream() {
            return playable;
        }
        let granule = packet.absgp_page();
        if granule == u64::MAX {
            return playable;
        }
        let target_frames = granule.saturating_sub(self.delay_padding.decoder_delay_frames());
        let available = target_frames.saturating_sub(already_emitted);
        playable.min(usize::try_from(available).unwrap_or(usize::MAX))
    }

    fn reset_to_start(&mut self) -> Result<(), DecoderError> {
        self.ensure_available()?;
        self.reader
            .as_mut()
            .ok_or_else(|| DecoderError::failure(DecoderFailureStage::Seek, "missing Ogg reader"))?
            .seek_bytes(SeekFrom::Start(0))
            .map_err(|_| {
                DecoderError::failure(DecoderFailureStage::Seek, "Ogg byte seek failed")
            })?;
        self.decoder = None;
        self.stream_format = None;
        self.channels = 0;
        self.pending.clear();
        self.pending_offset = 0;
        self.eof = false;
        self.read_headers()
    }
}

impl Default for OpusDecoder {
    fn default() -> Self {
        Self::new()
    }
}

impl Decoder for OpusDecoder {
    fn decode(
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
        if !output.len().is_multiple_of(self.channels) {
            return Err(DecoderError::InvalidOutput {
                samples_written: output.len(),
                capacity: output.len() - (output.len() % self.channels),
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

    fn seek(
        &mut self,
        source: &mut dyn RuntimeSource,
        target: SeekTarget,
    ) -> Result<SeekMetadata, DecoderError> {
        self.ensure_available()?;
        self.ensure_open(source)?;
        let frame = match target {
            SeekTarget::Frame(value) => value,
            SeekTarget::Microseconds(value) => value
                .checked_mul(u64::from(OPUS_OUTPUT_RATE_HZ))
                .and_then(|value| value.checked_div(1_000_000))
                .ok_or_else(|| {
                    DecoderError::failure(DecoderFailureStage::Seek, "Opus seek overflow")
                })?,
            SeekTarget::ByteOffset(_) => {
                return Err(DecoderError::unsupported(
                    "Ogg Opus byte-offset seek is not supported",
                ));
            }
        };
        self.reset_to_start()?;
        let mut discarded = 0_u64;
        let mut scratch = vec![0.0_f32; OPUS_MAX_PACKET_FRAMES * self.channels];
        loop {
            let packet = self
                .reader
                .as_mut()
                .ok_or_else(|| {
                    DecoderError::failure(DecoderFailureStage::Seek, "missing Ogg reader")
                })?
                .read_packet()
                .map_err(|error| map_ogg_error(&error))?
                .ok_or_else(|| {
                    DecoderError::failure(
                        DecoderFailureStage::Seek,
                        "seek target is outside Opus stream",
                    )
                })?;
            let frames = self
                .decoder
                .as_mut()
                .ok_or_else(|| {
                    DecoderError::failure(DecoderFailureStage::Seek, "missing Opus decoder")
                })?
                .decode_float(&packet.data, &mut scratch, false)
                .map_err(map_opus_error)?;
            let skip = self.pre_skip_remaining.min(frames);
            self.pre_skip_remaining -= skip;
            let playable = self.final_packet_frame_limit(&packet, frames, skip, discarded);
            let available_end =
                discarded.saturating_add(u64::try_from(playable).unwrap_or(u64::MAX));
            if available_end < frame {
                discarded = available_end;
                if packet.last_in_stream() {
                    return Err(DecoderError::failure(
                        DecoderFailureStage::Seek,
                        "seek target is outside Opus stream",
                    ));
                }
                continue;
            }
            let offset = usize::try_from(frame.saturating_sub(discarded)).map_err(|_| {
                DecoderError::failure(DecoderFailureStage::Seek, "Opus seek offset overflow")
            })?;
            let count_frames = playable.saturating_sub(offset);
            let start = (skip + offset) * self.channels;
            let count = count_frames * self.channels;
            self.pending.clear();
            self.pending
                .extend_from_slice(&scratch[start..start + count]);
            self.pending_offset = 0;
            self.emitted_frames = frame;
            self.eof = packet.last_in_stream() && count == 0;
            return Ok(SeekMetadata::new(self.delay_padding, None));
        }
    }

    fn reopen(&mut self) -> Result<(), DecoderError> {
        self.ensure_available()?;
        self.bytes = None;
        self.reader = None;
        self.decoder = None;
        self.stream_format = None;
        self.channels = 0;
        self.pre_skip_remaining = 0;
        self.delay_padding = DecoderDelayPadding::default();
        self.emitted_frames = 0;
        self.pending.clear();
        self.pending_offset = 0;
        self.eof = false;
        Ok(())
    }

    fn close(&mut self) -> Result<(), DecoderError> {
        self.bytes = None;
        self.reader = None;
        self.decoder = None;
        self.stream_format = None;
        self.pre_skip_remaining = 0;
        self.delay_padding = DecoderDelayPadding::default();
        self.emitted_frames = 0;
        self.pending.clear();
        self.pending_offset = 0;
        self.closed = true;
        Ok(())
    }

    fn cancel(&mut self) {
        self.cancelled = true;
        self.bytes = None;
        self.reader = None;
        self.decoder = None;
        self.pending.clear();
        self.pending_offset = 0;
    }
}

#[derive(Debug)]
struct OpusHeadSetup {
    channels: usize,
    pre_skip: usize,
    streams: usize,
    coupled_streams: usize,
    mapping: Vec<u8>,
}

fn parse_opus_head(data: &[u8]) -> Result<OpusHeadSetup, DecoderError> {
    if data.len() < 19 || &data[..8] != b"OpusHead" {
        return Err(DecoderError::unsupported("Ogg stream is not Opus"));
    }
    if data[8] != 1 {
        return Err(DecoderError::unsupported("unsupported OpusHead version"));
    }
    let channels = usize::from(data[9]);
    if !(1..=OPUS_MAX_CHANNELS).contains(&channels) {
        return Err(DecoderError::unsupported(
            "Opus channel count is outside 1-8",
        ));
    }
    let pre_skip = usize::from(u16::from_le_bytes([data[10], data[11]]));
    let mapping_family = data[18];
    match mapping_family {
        0 if channels <= 2 => Ok(OpusHeadSetup {
            channels,
            pre_skip,
            streams: 1,
            coupled_streams: usize::from(channels == 2),
            mapping: (0..channels)
                .map(|value| u8::try_from(value).expect("validated Opus channel count fits u8"))
                .collect(),
        }),
        1 => {
            let needed = 21_usize
                .checked_add(channels)
                .ok_or_else(|| DecoderError::corrupt("Opus channel mapping length overflow"))?;
            if data.len() < needed {
                return Err(DecoderError::corrupt("truncated Opus channel mapping"));
            }
            let streams = usize::from(data[19]);
            let coupled_streams = usize::from(data[20]);
            if streams == 0 || coupled_streams > streams {
                return Err(DecoderError::corrupt("invalid Opus stream mapping counts"));
            }
            let total = streams
                .checked_add(coupled_streams)
                .ok_or_else(|| DecoderError::corrupt("Opus stream mapping count overflow"))?;
            let mapping = data[21..needed].to_vec();
            if mapping
                .iter()
                .any(|&slot| slot != 255 && usize::from(slot) >= total)
            {
                return Err(DecoderError::corrupt(
                    "Opus channel mapping references an invalid stream",
                ));
            }
            Ok(OpusHeadSetup {
                channels,
                pre_skip,
                streams,
                coupled_streams,
                mapping,
            })
        }
        _ => Err(DecoderError::unsupported(
            "Opus channel mapping family is not supported",
        )),
    }
}

fn map_ogg_error(error: &OggReadError) -> DecoderError {
    DecoderError::corrupt(format!("Ogg packet rejected: {error:?}"))
}
