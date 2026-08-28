//! Typed failures for the platform-neutral audio runtime boundary.
//!
//! These values intentionally contain no filesystem path, operating-system
//! handle, URL, credential, or provider-specific state. Runtime adapters may
//! retain those values privately while an operation is active, but failures
//! crossing the audio boundary only expose a safe classification.

use std::{error::Error, fmt, io::ErrorKind};

/// The worker operation that produced a source I/O failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceOperation {
    /// Opening a local runtime resource.
    Open,
    /// Reading bytes from a runtime resource.
    Read,
    /// Moving the runtime resource cursor.
    Seek,
    /// Reopening a runtime resource after invalidation.
    Reopen,
    /// Closing a runtime resource.
    Close,
}

impl fmt::Display for SourceOperation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            Self::Open => "open",
            Self::Read => "read",
            Self::Seek => "seek",
            Self::Reopen => "reopen",
            Self::Close => "close",
        };
        formatter.write_str(name)
    }
}

/// A safe classification for a resource that cannot currently be used.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnavailableReason {
    /// The path resolved to something other than a regular file.
    NotARegularFile,
    /// The process lacks permission to use the resource.
    PermissionDenied,
    /// The resource exceeds the configured runtime size bound.
    TooLarge,
    /// The resource is closed and cannot accept more work.
    Closed,
    /// The resource is busy or otherwise temporarily unavailable.
    TemporarilyUnavailable,
    /// No runtime resource was supplied to the worker.
    NoResource,
}

impl fmt::Display for UnavailableReason {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            Self::NotARegularFile => "not a regular file",
            Self::PermissionDenied => "permission denied",
            Self::TooLarge => "resource exceeds the runtime size bound",
            Self::Closed => "resource is closed",
            Self::TemporarilyUnavailable => "resource is temporarily unavailable",
            Self::NoResource => "no runtime resource is available",
        };
        formatter.write_str(name)
    }
}

/// A classified failure from a runtime source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SourceError {
    /// The persistent locator no longer resolves to a readable local file.
    Missing,
    /// The caller cancelled the operation.
    Cancelled,
    /// The source has been closed and cannot be reused.
    Closed,
    /// The source exists but is not currently usable.
    Unavailable {
        /// A safe reason for the unavailable state.
        reason: UnavailableReason,
    },
    /// The caller attempted a read larger than the configured bound.
    ReadLimitExceeded {
        /// Number of bytes requested by the caller.
        requested: usize,
        /// Maximum bytes permitted for one read.
        maximum: usize,
    },
    /// The requested cursor position is outside the bounded source.
    SeekOutOfBounds {
        /// The rejected absolute byte position.
        position: u64,
        /// The exclusive upper bound of the source.
        length: u64,
    },
    /// A source limit was configured with an invalid value.
    InvalidLimit {
        /// Name of the invalid limit.
        name: &'static str,
        /// Supplied numeric value.
        value: u64,
    },
    /// An operating-system error without exposing a path or handle.
    Io {
        /// Operation being performed when the error occurred.
        operation: SourceOperation,
        /// Stable standard-library error classification.
        kind: ErrorKind,
    },
}

impl fmt::Display for SourceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Missing => formatter.write_str("local source is missing"),
            Self::Cancelled => formatter.write_str("source operation was cancelled"),
            Self::Closed => formatter.write_str("local source is closed"),
            Self::Unavailable { reason } => write!(formatter, "source unavailable: {reason}"),
            Self::ReadLimitExceeded { requested, maximum } => write!(
                formatter,
                "source read of {requested} bytes exceeds the {maximum}-byte bound"
            ),
            Self::SeekOutOfBounds { position, length } => write!(
                formatter,
                "source seek position {position} is outside length {length}"
            ),
            Self::InvalidLimit { name, value } => {
                write!(formatter, "source limit {name} has invalid value {value}")
            }
            Self::Io { operation, kind } => {
                write!(formatter, "source {operation} failed with {kind:?}")
            }
        }
    }
}

impl Error for SourceError {}

/// The worker phase in which a decoder failure occurred.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecoderFailureStage {
    /// Decoder initialization or source binding.
    Open,
    /// Reading encoded bytes from the source.
    Read,
    /// Turning encoded bytes into PCM.
    Decode,
    /// Seeking decoder state.
    Seek,
    /// Rebuilding decoder state after a source reopen.
    Reopen,
    /// Releasing decoder state.
    Close,
}

impl fmt::Display for DecoderFailureStage {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            Self::Open => "open",
            Self::Read => "read",
            Self::Decode => "decode",
            Self::Seek => "seek",
            Self::Reopen => "reopen",
            Self::Close => "close",
        };
        formatter.write_str(name)
    }
}

/// A classified failure from a format-neutral decoder worker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DecoderError {
    /// The input format is not supported by the selected decoder.
    Unsupported {
        /// A safe format label, such as a container or codec name.
        format: String,
    },
    /// The input is recognized but structurally invalid or truncated.
    Corrupt {
        /// A concise, non-sensitive diagnostic.
        detail: String,
    },
    /// The local source disappeared before or during decoding.
    Missing,
    /// The caller cancelled the worker operation.
    Cancelled,
    /// The decoder has already been closed.
    Closed,
    /// The source or decoder is temporarily unavailable.
    Unavailable {
        /// A safe reason for the unavailable state.
        reason: UnavailableReason,
    },
    /// A source failure that does not fit the higher-level classifications.
    Source(SourceError),
    /// A seek coordinator rejected a stale or invalid operation.
    Seek(crate::errors::SeekError),
    /// The decoder failed while processing an otherwise accepted operation.
    DecoderFailure {
        /// Worker phase at which the decoder failed.
        stage: DecoderFailureStage,
        /// A concise, non-sensitive diagnostic.
        detail: String,
    },
    /// A decoder returned more samples than the caller supplied.
    InvalidOutput {
        /// Number of samples reported by the decoder.
        samples_written: usize,
        /// Capacity of the supplied output buffer.
        capacity: usize,
    },
    /// The worker was asked to perform an operation in an incompatible state.
    InvalidState {
        /// Operation that was rejected.
        operation: &'static str,
        /// Stable state label without coupling this module to the worker type.
        state: &'static str,
    },
    /// A worker configuration violates a bounded-runtime invariant.
    InvalidConfiguration {
        /// Name of the invalid field.
        field: &'static str,
        /// Supplied value.
        value: usize,
    },
}

impl DecoderError {
    /// Creates an unsupported-format error without accepting arbitrary context.
    #[must_use]
    pub fn unsupported(format: impl Into<String>) -> Self {
        Self::Unsupported {
            format: format.into(),
        }
    }

    /// Creates a corrupt-input error.
    #[must_use]
    pub fn corrupt(detail: impl Into<String>) -> Self {
        Self::Corrupt {
            detail: detail.into(),
        }
    }

    /// Creates a typed decoder-failure error.
    #[must_use]
    pub fn failure(stage: DecoderFailureStage, detail: impl Into<String>) -> Self {
        Self::DecoderFailure {
            stage,
            detail: detail.into(),
        }
    }
}

impl fmt::Display for DecoderError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unsupported { format } => write!(formatter, "unsupported audio format {format}"),
            Self::Corrupt { detail } => write!(formatter, "corrupt audio input: {detail}"),
            Self::Missing => formatter.write_str("decoder source is missing"),
            Self::Cancelled => formatter.write_str("decoder operation was cancelled"),
            Self::Closed => formatter.write_str("decoder is closed"),
            Self::Unavailable { reason } => write!(formatter, "decoder unavailable: {reason}"),
            Self::Source(error) => error.fmt(formatter),
            Self::Seek(error) => error.fmt(formatter),
            Self::DecoderFailure { stage, detail } => {
                write!(formatter, "decoder {stage} failed: {detail}")
            }
            Self::InvalidOutput {
                samples_written,
                capacity,
            } => write!(
                formatter,
                "decoder reported {samples_written} samples for a {capacity}-sample buffer"
            ),
            Self::InvalidState { operation, state } => {
                write!(formatter, "cannot {operation} decoder in {state} state")
            }
            Self::InvalidConfiguration { field, value } => {
                write!(
                    formatter,
                    "decoder configuration {field} has invalid value {value}"
                )
            }
        }
    }
}

impl Error for DecoderError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Source(error) => Some(error),
            Self::Seek(error) => Some(error),
            _ => None,
        }
    }
}

impl From<SourceError> for DecoderError {
    fn from(error: SourceError) -> Self {
        match error {
            SourceError::Missing => Self::Missing,
            SourceError::Cancelled => Self::Cancelled,
            SourceError::Closed => Self::Closed,
            SourceError::Unavailable { reason } => Self::Unavailable { reason },
            other => Self::Source(other),
        }
    }
}

/// A seek-coordination failure that prevents a worker result from committing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SeekError {
    /// The seek was cancelled before it became effective.
    Cancelled,
    /// The worker or source is already closed.
    Closed,
    /// The requested generation counter would overflow.
    GenerationExhausted,
    /// A result arrived for a request that is no longer active.
    StaleRequest {
        /// Request ID carried by the stale result.
        request_id: u64,
    },
    /// A result carries a retired buffer generation.
    StaleGeneration {
        /// Generation expected by the consumer.
        expected: u64,
        /// Generation carried by the stale result.
        actual: u64,
    },
    /// A source operation failed while executing the seek.
    Source(SourceError),
}

impl fmt::Display for SeekError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Cancelled => formatter.write_str("seek was cancelled"),
            Self::Closed => formatter.write_str("seek target is closed"),
            Self::GenerationExhausted => formatter.write_str("buffer generation counter exhausted"),
            Self::StaleRequest { request_id } => {
                write!(formatter, "seek request {request_id} is stale")
            }
            Self::StaleGeneration { expected, actual } => write!(
                formatter,
                "buffer generation {actual} is stale; current generation is {expected}"
            ),
            Self::Source(error) => error.fmt(formatter),
        }
    }
}

impl Error for SeekError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Source(error) => Some(error),
            _ => None,
        }
    }
}

impl From<SourceError> for SeekError {
    fn from(error: SourceError) -> Self {
        match error {
            SourceError::Cancelled => Self::Cancelled,
            SourceError::Closed => Self::Closed,
            other => Self::Source(other),
        }
    }
}

impl From<SeekError> for DecoderError {
    fn from(error: SeekError) -> Self {
        match error {
            SeekError::Cancelled => Self::Cancelled,
            SeekError::Closed => Self::Closed,
            other => Self::Seek(other),
        }
    }
}

/// A single error type for callers that do not need to distinguish source,
/// decoder, and seek layers themselves.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AudioRuntimeError {
    /// The input format is not supported.
    Unsupported {
        /// Safe format label.
        format: String,
    },
    /// The input is structurally corrupt.
    Corrupt {
        /// Safe diagnostic.
        detail: String,
    },
    /// The source disappeared.
    Missing,
    /// The operation was cancelled.
    Cancelled,
    /// The resource is unavailable.
    Unavailable {
        /// Safe unavailable reason.
        reason: UnavailableReason,
    },
    /// A decoder-specific failure.
    DecoderFailure {
        /// Worker phase.
        stage: DecoderFailureStage,
        /// Safe diagnostic.
        detail: String,
    },
    /// A source-layer failure.
    Source(SourceError),
    /// A decoder-layer failure.
    Decoder(DecoderError),
    /// A seek/generation failure.
    Seek(SeekError),
}

/// Short alias for the runtime error boundary.
pub type AudioError = AudioRuntimeError;

impl fmt::Display for AudioRuntimeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unsupported { format } => write!(formatter, "unsupported audio format {format}"),
            Self::Corrupt { detail } => write!(formatter, "corrupt audio input: {detail}"),
            Self::Missing => formatter.write_str("audio source is missing"),
            Self::Cancelled => formatter.write_str("audio operation was cancelled"),
            Self::Unavailable { reason } => write!(formatter, "audio unavailable: {reason}"),
            Self::DecoderFailure { stage, detail } => {
                write!(formatter, "decoder {stage} failed: {detail}")
            }
            Self::Source(error) => error.fmt(formatter),
            Self::Decoder(error) => error.fmt(formatter),
            Self::Seek(error) => error.fmt(formatter),
        }
    }
}

impl Error for AudioRuntimeError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Source(error) => Some(error),
            Self::Decoder(error) => Some(error),
            Self::Seek(error) => Some(error),
            _ => None,
        }
    }
}

impl From<SourceError> for AudioRuntimeError {
    fn from(error: SourceError) -> Self {
        match error {
            SourceError::Missing => Self::Missing,
            SourceError::Cancelled => Self::Cancelled,
            SourceError::Unavailable { reason } => Self::Unavailable { reason },
            other => Self::Source(other),
        }
    }
}

impl From<DecoderError> for AudioRuntimeError {
    fn from(error: DecoderError) -> Self {
        match error {
            DecoderError::Unsupported { format } => Self::Unsupported { format },
            DecoderError::Corrupt { detail } => Self::Corrupt { detail },
            DecoderError::Missing => Self::Missing,
            DecoderError::Cancelled => Self::Cancelled,
            DecoderError::Unavailable { reason } => Self::Unavailable { reason },
            DecoderError::DecoderFailure { stage, detail } => {
                Self::DecoderFailure { stage, detail }
            }
            other => Self::Decoder(other),
        }
    }
}

impl From<SeekError> for AudioRuntimeError {
    fn from(error: SeekError) -> Self {
        match error {
            SeekError::Cancelled => Self::Cancelled,
            SeekError::Closed => Self::Unavailable {
                reason: UnavailableReason::Closed,
            },
            other => Self::Seek(other),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        AudioRuntimeError, DecoderError, DecoderFailureStage, SeekError, SourceError,
        SourceOperation, UnavailableReason,
    };
    use std::io::ErrorKind;

    #[test]
    fn source_errors_have_stable_safe_display() {
        let error = SourceError::Io {
            operation: SourceOperation::Read,
            kind: ErrorKind::PermissionDenied,
        };

        assert_eq!(
            error.to_string(),
            "source read failed with PermissionDenied"
        );
        assert!(!error.to_string().contains("C:\\"));
    }

    #[test]
    fn source_classifications_map_to_decoder_classifications() {
        assert_eq!(
            DecoderError::from(SourceError::Missing),
            DecoderError::Missing
        );
        assert_eq!(
            DecoderError::from(SourceError::Unavailable {
                reason: UnavailableReason::PermissionDenied,
            }),
            DecoderError::Unavailable {
                reason: UnavailableReason::PermissionDenied,
            }
        );
        assert_eq!(
            DecoderError::from(SourceError::Cancelled),
            DecoderError::Cancelled
        );
    }

    #[test]
    fn decoder_failure_maps_to_the_unified_runtime_error() {
        let error = AudioRuntimeError::from(DecoderError::failure(
            DecoderFailureStage::Decode,
            "fixture failure",
        ));

        assert_eq!(
            error,
            AudioRuntimeError::DecoderFailure {
                stage: DecoderFailureStage::Decode,
                detail: "fixture failure".to_owned(),
            }
        );
    }

    #[test]
    fn stale_generation_and_request_are_distinct() {
        assert_ne!(
            SeekError::StaleRequest { request_id: 1 },
            SeekError::StaleGeneration {
                expected: 2,
                actual: 1,
            }
        );
    }
}
