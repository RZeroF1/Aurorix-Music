//! Platform-neutral audio source, decoder, PCM, and realtime boundaries.

pub mod buffer;
pub mod capture_sink;
pub mod decoder;
pub mod decoders;
pub mod errors;
pub mod format;
pub mod output_report;
pub mod pcm;
pub mod realtime;
pub mod recovery;
pub mod seek;
pub mod source;
