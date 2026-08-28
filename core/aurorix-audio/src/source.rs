//! Runtime-only local source capabilities.
//!
//! A source owns an opened file only for the lifetime of an active worker. The
//! path and handle are private runtime state; this module provides no
//! persistence, serialization, provider credential, or database representation.

use crate::errors::{SourceError, SourceOperation, UnavailableReason};
use std::{
    fs::{self, File},
    io::{Read, Seek, SeekFrom},
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

/// Default maximum number of encoded bytes accepted by one source read.
pub const DEFAULT_MAX_READ_BYTES: usize = 64 * 1024;

/// Runtime bounds applied while opening and reading a local source.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SourceLimits {
    max_read_bytes: usize,
    max_source_bytes: u64,
}

impl SourceLimits {
    /// Creates validated source bounds.
    ///
    /// A zero read bound or zero source bound cannot make forward progress and
    /// is rejected before a runtime handle is opened.
    ///
    /// # Errors
    ///
    /// Returns [`SourceError::InvalidLimit`] when either bound is zero.
    pub fn new(max_read_bytes: usize, max_source_bytes: u64) -> Result<Self, SourceError> {
        if max_read_bytes == 0 {
            return Err(SourceError::InvalidLimit {
                name: "max_read_bytes",
                value: 0,
            });
        }
        if max_source_bytes == 0 {
            return Err(SourceError::InvalidLimit {
                name: "max_source_bytes",
                value: 0,
            });
        }
        Ok(Self {
            max_read_bytes,
            max_source_bytes,
        })
    }

    /// Returns the maximum bytes accepted by one read.
    #[must_use]
    pub const fn max_read_bytes(self) -> usize {
        self.max_read_bytes
    }

    /// Returns the maximum source length accepted at open/reopen time.
    #[must_use]
    pub const fn max_source_bytes(self) -> u64 {
        self.max_source_bytes
    }
}

impl Default for SourceLimits {
    fn default() -> Self {
        Self {
            max_read_bytes: DEFAULT_MAX_READ_BYTES,
            max_source_bytes: u64::MAX,
        }
    }
}

/// A cooperative one-shot cancellation token for worker-side source work.
#[derive(Clone, Debug, Default)]
pub struct CancellationToken {
    cancelled: Arc<AtomicBool>,
}

impl CancellationToken {
    /// Creates a non-cancelled token.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Marks the token cancelled. Cancellation is intentionally one-shot.
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
    }

    /// Returns whether cancellation has been requested.
    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Acquire)
    }
}

/// The result of one bounded source read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SourceRead {
    bytes_read: usize,
    end_of_stream: bool,
}

impl SourceRead {
    /// Creates a read result for source adapters and test doubles.
    #[must_use]
    pub const fn new(bytes_read: usize, end_of_stream: bool) -> Self {
        Self {
            bytes_read,
            end_of_stream,
        }
    }

    /// Returns the number of bytes written into the caller's buffer.
    #[must_use]
    pub const fn bytes_read(self) -> usize {
        self.bytes_read
    }

    /// Returns whether the source reached its end.
    #[must_use]
    pub const fn end_of_stream(self) -> bool {
        self.end_of_stream
    }
}

/// Runtime source capability consumed by a decoder worker.
pub trait RuntimeSource: Send {
    /// Reads at most the configured source bound into `destination`.
    ///
    /// # Errors
    ///
    /// Returns a typed failure when the source is cancelled, closed, missing,
    /// unavailable, or when `destination` exceeds its configured read bound.
    fn read_bounded(&mut self, destination: &mut [u8]) -> Result<SourceRead, SourceError>;

    /// Moves the runtime cursor within the bounded source.
    ///
    /// # Errors
    ///
    /// Returns a typed failure when the source is cancelled, closed, or the
    /// requested position is outside the source bound.
    fn seek(&mut self, position: SeekFrom) -> Result<u64, SourceError>;

    /// Reopens the same runtime locator at byte position zero.
    ///
    /// # Errors
    ///
    /// Returns a typed failure when the source was cancelled or closed, or the
    /// runtime locator no longer resolves to an acceptable local resource.
    fn reopen(&mut self) -> Result<(), SourceError>;

    /// Returns the current runtime cursor position.
    ///
    /// # Errors
    ///
    /// Returns a typed failure when the source is cancelled or closed.
    fn position(&mut self) -> Result<u64, SourceError>;

    /// Returns the bounded source length when known.
    fn len(&self) -> Option<u64>;

    /// Returns whether the known source length is zero.
    #[must_use]
    fn is_empty(&self) -> Option<bool> {
        self.len().map(|length| length == 0)
    }

    /// Closes the runtime source. Closing is idempotent.
    ///
    /// # Errors
    ///
    /// Implementations return a typed failure only when an explicit close
    /// operation cannot retire their runtime resource.
    fn close(&mut self) -> Result<(), SourceError>;

    /// Requests cooperative cancellation and retires the runtime handle.
    fn cancel(&mut self);

    /// Returns whether cancellation was requested.
    fn is_cancelled(&self) -> bool;
}

/// Short compatibility name for the runtime source capability.
pub use RuntimeSource as Source;

/// A local file source with bounded worker-side operations.
pub struct LocalFileSource {
    // These fields are deliberately private runtime state. There is no
    // serialization or persistence implementation for this type.
    path: PathBuf,
    file: Option<File>,
    length: u64,
    position: u64,
    limits: SourceLimits,
    cancellation: CancellationToken,
    closed: bool,
}

/// Compatibility name used by the Gate 2 source boundary.
pub type LocalSource = LocalFileSource;

impl LocalFileSource {
    /// Opens a regular local file using the default runtime bounds.
    ///
    /// # Errors
    ///
    /// Returns a typed [`SourceError`] without retaining an OS error path in
    /// the error value.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, SourceError> {
        Self::open_with_limits(path, SourceLimits::default())
    }

    /// Opens a regular local file with explicit runtime bounds.
    ///
    /// # Errors
    ///
    /// Returns [`SourceError::Missing`] when the path does not resolve,
    /// [`SourceError::Unavailable`] for access/type/size failures, or a typed
    /// I/O classification for other operating-system failures.
    pub fn open_with_limits(
        path: impl AsRef<Path>,
        limits: SourceLimits,
    ) -> Result<Self, SourceError> {
        let path = path.as_ref().to_path_buf();
        let metadata =
            fs::metadata(&path).map_err(|error| map_io(SourceOperation::Open, &error))?;
        if !metadata.is_file() {
            return Err(SourceError::Unavailable {
                reason: UnavailableReason::NotARegularFile,
            });
        }
        if metadata.len() > limits.max_source_bytes() {
            return Err(SourceError::Unavailable {
                reason: UnavailableReason::TooLarge,
            });
        }
        let file = File::open(&path).map_err(|error| map_io(SourceOperation::Open, &error))?;
        let length = metadata.len();

        Ok(Self {
            path,
            file: Some(file),
            length,
            position: 0,
            limits,
            cancellation: CancellationToken::new(),
            closed: false,
        })
    }

    /// Returns the active runtime bounds.
    #[must_use]
    pub const fn limits(&self) -> SourceLimits {
        self.limits
    }

    /// Returns a clone of the runtime cancellation token.
    #[must_use]
    pub fn cancellation_token(&self) -> CancellationToken {
        self.cancellation.clone()
    }

    /// Returns the source length discovered at open/reopen time.
    #[must_use]
    pub const fn length(&self) -> u64 {
        self.length
    }

    /// Returns the tracked runtime cursor without performing I/O.
    #[must_use]
    pub const fn current_position(&self) -> u64 {
        self.position
    }

    /// Returns whether the runtime handle has been closed.
    #[must_use]
    pub fn is_closed(&self) -> bool {
        self.closed || self.cancellation.is_cancelled()
    }

    fn ensure_active(&self) -> Result<(), SourceError> {
        if self.cancellation.is_cancelled() {
            return Err(SourceError::Cancelled);
        }
        if self.closed || self.file.is_none() {
            return Err(SourceError::Closed);
        }
        Ok(())
    }

    fn retire_if_cancelled(&mut self) {
        if self.cancellation.is_cancelled() {
            self.file.take();
            self.closed = true;
        }
    }

    fn validate_target(&self, target: u64) -> Result<(), SourceError> {
        if target > self.length {
            return Err(SourceError::SeekOutOfBounds {
                position: target,
                length: self.length,
            });
        }
        Ok(())
    }

    fn reopen_file(&mut self) -> Result<(), SourceError> {
        self.retire_if_cancelled();
        self.ensure_active()?;
        let file =
            File::open(&self.path).map_err(|error| map_io(SourceOperation::Reopen, &error))?;
        let length = source_length(&file, &self.limits).map_err(|error| match error {
            SourceError::Io { kind, .. } => SourceError::Io {
                operation: SourceOperation::Reopen,
                kind,
            },
            other => other,
        })?;
        self.file = Some(file);
        self.length = length;
        self.position = 0;
        Ok(())
    }
}

impl RuntimeSource for LocalFileSource {
    fn read_bounded(&mut self, destination: &mut [u8]) -> Result<SourceRead, SourceError> {
        self.retire_if_cancelled();
        self.ensure_active()?;
        if destination.len() > self.limits.max_read_bytes() {
            return Err(SourceError::ReadLimitExceeded {
                requested: destination.len(),
                maximum: self.limits.max_read_bytes(),
            });
        }
        if destination.is_empty() {
            return Ok(SourceRead::new(0, self.position >= self.length));
        }
        if self.position >= self.length {
            return Ok(SourceRead::new(0, true));
        }

        let remaining = self.length - self.position;
        let remaining_bound = usize::try_from(remaining).unwrap_or(usize::MAX);
        let read_length = destination.len().min(remaining_bound);

        let file = self.file.as_mut().ok_or(SourceError::Closed)?;
        let bytes_read = file
            .read(&mut destination[..read_length])
            .map_err(|error| map_io(SourceOperation::Read, &error))?;
        if self.cancellation.is_cancelled() {
            return Err(SourceError::Cancelled);
        }
        let bytes_read_u64 = u64::try_from(bytes_read).map_err(|_| SourceError::Unavailable {
            reason: UnavailableReason::TooLarge,
        })?;
        self.position =
            self.position
                .checked_add(bytes_read_u64)
                .ok_or(SourceError::Unavailable {
                    reason: UnavailableReason::TooLarge,
                })?;

        Ok(SourceRead::new(
            bytes_read,
            bytes_read == 0 || self.position >= self.length,
        ))
    }

    fn seek(&mut self, position: SeekFrom) -> Result<u64, SourceError> {
        self.retire_if_cancelled();
        self.ensure_active()?;
        let target = resolve_seek(position, self.position, self.length)?;
        self.validate_target(target)?;
        let file = self.file.as_mut().ok_or(SourceError::Closed)?;
        let actual = file
            .seek(SeekFrom::Start(target))
            .map_err(|error| map_io(SourceOperation::Seek, &error))?;
        self.position = actual;
        Ok(actual)
    }

    fn reopen(&mut self) -> Result<(), SourceError> {
        self.reopen_file()
    }

    fn position(&mut self) -> Result<u64, SourceError> {
        self.retire_if_cancelled();
        self.ensure_active()?;
        Ok(self.position)
    }

    fn len(&self) -> Option<u64> {
        Some(self.length)
    }

    fn close(&mut self) -> Result<(), SourceError> {
        if self.closed {
            return Ok(());
        }
        self.file.take();
        self.closed = true;
        Ok(())
    }

    fn cancel(&mut self) {
        self.cancellation.cancel();
        self.file.take();
        self.closed = true;
    }

    fn is_cancelled(&self) -> bool {
        self.cancellation.is_cancelled()
    }
}

fn source_length(file: &File, limits: &SourceLimits) -> Result<u64, SourceError> {
    let metadata = file
        .metadata()
        .map_err(|error| map_io(SourceOperation::Open, &error))?;
    if !metadata.is_file() {
        return Err(SourceError::Unavailable {
            reason: UnavailableReason::NotARegularFile,
        });
    }
    let length = metadata.len();
    if length > limits.max_source_bytes() {
        return Err(SourceError::Unavailable {
            reason: UnavailableReason::TooLarge,
        });
    }
    Ok(length)
}

fn resolve_seek(position: SeekFrom, current: u64, length: u64) -> Result<u64, SourceError> {
    let target = match position {
        SeekFrom::Start(offset) => Some(offset),
        SeekFrom::Current(offset) if offset >= 0 => current.checked_add(offset.cast_unsigned()),
        SeekFrom::Current(offset) => current.checked_sub(offset.unsigned_abs()),
        SeekFrom::End(offset) if offset >= 0 => length.checked_add(offset.cast_unsigned()),
        SeekFrom::End(offset) => length.checked_sub(offset.unsigned_abs()),
    }
    .ok_or(SourceError::SeekOutOfBounds {
        position: u64::MAX,
        length,
    })?;
    Ok(target)
}

fn map_io(operation: SourceOperation, error: &std::io::Error) -> SourceError {
    match error.kind() {
        std::io::ErrorKind::NotFound => SourceError::Missing,
        std::io::ErrorKind::PermissionDenied => SourceError::Unavailable {
            reason: UnavailableReason::PermissionDenied,
        },
        _ => SourceError::Io {
            operation,
            kind: error.kind(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::{
        CancellationToken, DEFAULT_MAX_READ_BYTES, LocalFileSource, RuntimeSource, SourceLimits,
    };
    use crate::errors::{SourceError, UnavailableReason};
    use std::{
        fs,
        io::{SeekFrom, Write},
        path::PathBuf,
        time::{SystemTime, UNIX_EPOCH},
    };

    fn fixture_path(label: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock must be after epoch")
            .as_nanos();
        std::env::temp_dir().join(format!("aurorix-audio-{label}-{nanos}.bin"))
    }

    fn fixture(label: &str, bytes: &[u8]) -> PathBuf {
        let path = fixture_path(label);
        let mut file = fs::File::create(&path).expect("fixture should be creatable");
        file.write_all(bytes).expect("fixture should be writable");
        path
    }

    #[test]
    fn default_read_bound_is_explicit_and_reads_are_bounded() {
        assert_eq!(DEFAULT_MAX_READ_BYTES, 64 * 1024);
        let path = fixture("bounded", b"012345");
        let limits = SourceLimits::new(2, 16).expect("valid bounds");
        let mut source = LocalFileSource::open_with_limits(&path, limits).expect("source opens");
        let mut too_large = [0_u8; 3];
        assert!(matches!(
            source.read_bounded(&mut too_large),
            Err(SourceError::ReadLimitExceeded {
                requested: 3,
                maximum: 2
            })
        ));

        let mut chunk = [0_u8; 2];
        let first = source.read_bounded(&mut chunk).expect("bounded read");
        assert_eq!(first.bytes_read(), 2);
        assert!(!first.end_of_stream());
        assert_eq!(&chunk, b"01");
        assert_eq!(source.current_position(), 2);
        source.close().expect("close is successful");
        fs::remove_file(path).expect("fixture cleanup");
    }

    #[test]
    fn seek_reopen_and_eof_are_deterministic() {
        let path = fixture("seek", b"abcdef");
        let mut source = LocalFileSource::open(&path).expect("source opens");
        assert_eq!(source.seek(SeekFrom::Start(4)).expect("seek works"), 4);
        let mut chunk = [0_u8; 2];
        let result = source.read_bounded(&mut chunk).expect("read works");
        assert_eq!(&chunk, b"ef");
        assert!(result.end_of_stream());
        assert!(matches!(
            source.seek(SeekFrom::Start(7)),
            Err(SourceError::SeekOutOfBounds {
                position: 7,
                length: 6
            })
        ));
        source.reopen().expect("reopen works");
        assert_eq!(source.current_position(), 0);
        let mut first = [0_u8; 1];
        source.read_bounded(&mut first).expect("read after reopen");
        assert_eq!(first, [b'a']);
        fs::remove_file(path).expect("fixture cleanup");
    }

    #[test]
    fn read_does_not_cross_the_length_observed_when_the_source_opened() {
        let path = fixture("length-bound", b"ab");
        let mut source = LocalFileSource::open(&path).expect("source opens");
        let mut append = fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .expect("fixture should be appendable");
        append.write_all(b"cd").expect("fixture append should work");
        drop(append);

        let mut output = [0_u8; 4];
        let read = source.read_bounded(&mut output).expect("bounded read");
        assert_eq!(read.bytes_read(), 2);
        assert!(read.end_of_stream());
        assert_eq!(&output[..2], b"ab");
        assert_eq!(source.current_position(), 2);
        fs::remove_file(path).expect("fixture cleanup");
    }

    #[test]
    fn cancellation_retires_handle_and_is_not_reversible() {
        let path = fixture("cancel", b"data");
        let mut source = LocalFileSource::open(&path).expect("source opens");
        let token = source.cancellation_token();
        token.cancel();
        let mut output = [0_u8; 1];
        assert_eq!(
            source.read_bounded(&mut output),
            Err(SourceError::Cancelled)
        );
        assert!(source.is_closed());
        source.cancel();
        assert!(source.is_cancelled());
        fs::remove_file(path).expect("fixture cleanup");

        let independent = CancellationToken::new();
        assert!(!independent.is_cancelled());
        independent.cancel();
        assert!(independent.is_cancelled());
    }

    #[test]
    fn missing_and_non_file_resources_are_typed() {
        let missing = fixture_path("missing");
        assert!(matches!(
            LocalFileSource::open(missing),
            Err(SourceError::Missing)
        ));

        let directory = std::env::temp_dir().join("aurorix-audio-directory-fixture");
        fs::create_dir_all(&directory).expect("directory fixture should be creatable");
        assert!(matches!(
            LocalFileSource::open(&directory),
            Err(SourceError::Unavailable {
                reason: UnavailableReason::NotARegularFile,
            })
        ));
        fs::remove_dir(&directory).expect("directory cleanup");
    }

    #[test]
    fn close_is_idempotent_but_does_not_reopen_a_retired_source() {
        let path = fixture("close", b"x");
        let mut source = LocalFileSource::open(&path).expect("source opens");
        source.close().expect("first close works");
        source.close().expect("second close works");
        assert_eq!(source.reopen(), Err(SourceError::Closed));
        fs::remove_file(path).expect("fixture cleanup");
    }
}
