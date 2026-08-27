//! Pure values and integrity checks for Sync snapshot bootstrap.
//!
//! This module carries no Cloud client, persistence handle, recovery workflow,
//! cryptographic key material, or database migration behavior. A caller must
//! validate a manifest and its received chunks before staging any snapshot.

#![allow(clippy::struct_field_names, clippy::too_many_arguments)]

use std::{error::Error, fmt};

use sha2::{Digest, Sha256};
use uuid::Uuid;

/// The largest supported snapshot chunk, in bytes.
pub const MAX_SNAPSHOT_CHUNK_SIZE_BYTES: u32 = 10 * 1024 * 1024;

/// The largest number of descriptors accepted in one snapshot manifest.
pub const MAX_SNAPSHOT_CHUNKS: u32 = 100_000;

/// The maximum number of Unicode scalar values in a snapshot schema version.
pub const MAX_SNAPSHOT_SCHEMA_VERSION_CHARS: usize = 64;

/// The maximum number of Unicode scalar values in a recovery message.
pub const MAX_RECOVERY_MESSAGE_CHARS: usize = 1_024;

/// A committed position in one immutable Sync history.
///
/// A snapshot ID is intentionally not part of this value: it identifies
/// bootstrap provenance, not a point in the ordering coordinate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SyncCursor {
    sync_epoch: Uuid,
    revision: u64,
}

impl SyncCursor {
    /// Creates a cursor from the server epoch and committed revision.
    #[must_use]
    pub const fn new(sync_epoch: Uuid, revision: u64) -> Self {
        Self {
            sync_epoch,
            revision,
        }
    }

    /// Returns the history epoch this cursor belongs to.
    #[must_use]
    pub const fn sync_epoch(self) -> Uuid {
        self.sync_epoch
    }

    /// Returns the committed revision within [`Self::sync_epoch`].
    #[must_use]
    pub const fn revision(self) -> u64 {
        self.revision
    }
}

/// Whether the server is available for ordinary incremental synchronization.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RecoveryStatus {
    /// The server accepts ordinary incremental push and pull requests.
    Normal,
    /// The server has temporarily frozen writes while recovery is coordinated.
    WriteFrozen,
    /// The client must bootstrap a verified snapshot before it can continue.
    RecoveryRequired,
}

impl RecoveryStatus {
    /// Returns the state entered when a server epoch does not match the cursor.
    #[must_use]
    pub const fn for_epoch_mismatch() -> Self {
        Self::RecoveryRequired
    }

    /// Returns whether ordinary push and pull may continue in this state.
    ///
    /// Both a frozen server and an epoch mismatch stop ordinary synchronization.
    #[must_use]
    pub const fn allows_ordinary_push_pull(self) -> bool {
        matches!(self, Self::Normal)
    }
}

/// A public recovery policy that a client may display and act on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RecoveryPolicy {
    /// No recovery action is currently required.
    None,
    /// Install a snapshot, then rebase permitted pending outbox operations.
    BootstrapRebase,
    /// A separately reviewed client contribution is permitted by policy.
    ReviewedClientContribution,
}

/// An error while constructing display-safe public recovery text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DisplaySafeMessageError {
    /// The message exceeded [`MAX_RECOVERY_MESSAGE_CHARS`].
    TooLong {
        /// The number of Unicode scalar values supplied.
        actual: usize,
        /// The largest accepted number of Unicode scalar values.
        maximum: usize,
    },
    /// The message contained a control character.
    ContainsControlCharacter,
}

impl fmt::Display for DisplaySafeMessageError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooLong { actual, maximum } => {
                write!(
                    formatter,
                    "recovery message has {actual} characters; maximum is {maximum}"
                )
            }
            Self::ContainsControlCharacter => {
                formatter.write_str("recovery message must not contain control characters")
            }
        }
    }
}

impl Error for DisplaySafeMessageError {}

/// A bounded message suitable for direct display in a client recovery surface.
///
/// This type excludes control characters and contains no diagnostic, credential,
/// or transport fields. It does not try to infer the truth of the recovery
/// statement; that remains a server-side policy decision.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct DisplaySafeMessage(String);

impl DisplaySafeMessage {
    /// Creates a bounded, control-character-free recovery message.
    ///
    /// # Errors
    ///
    /// Returns [`DisplaySafeMessageError`] when the message is longer than
    /// [`MAX_RECOVERY_MESSAGE_CHARS`] or includes a control character.
    pub fn new(value: impl Into<String>) -> Result<Self, DisplaySafeMessageError> {
        let value = value.into();
        let character_count = value.chars().count();
        if character_count > MAX_RECOVERY_MESSAGE_CHARS {
            return Err(DisplaySafeMessageError::TooLong {
                actual: character_count,
                maximum: MAX_RECOVERY_MESSAGE_CHARS,
            });
        }
        if value.chars().any(char::is_control) {
            return Err(DisplaySafeMessageError::ContainsControlCharacter);
        }

        Ok(Self(value))
    }

    /// Returns the display-safe message.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Public recovery facts accompanying a Sync head or recovery response.
///
/// The fields are deliberately limited to public display-safe recovery facts;
/// this value has no credential, key, host, account, or diagnostic payload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecoveryDisclosure {
    generation: Option<Uuid>,
    policy: RecoveryPolicy,
    acknowledged_data_loss: bool,
    lost_through_revision: Option<u64>,
    message: Option<DisplaySafeMessage>,
}

impl RecoveryDisclosure {
    /// Creates a public recovery disclosure from already display-safe values.
    #[must_use]
    pub fn new(
        generation: Option<Uuid>,
        policy: RecoveryPolicy,
        acknowledged_data_loss: bool,
        lost_through_revision: Option<u64>,
        message: Option<DisplaySafeMessage>,
    ) -> Self {
        Self {
            generation,
            policy,
            acknowledged_data_loss,
            lost_through_revision,
            message,
        }
    }

    /// Returns the public recovery generation, when one has been published.
    #[must_use]
    pub const fn generation(&self) -> Option<Uuid> {
        self.generation
    }

    /// Returns the policy clients must apply after recovery.
    #[must_use]
    pub const fn policy(&self) -> RecoveryPolicy {
        self.policy
    }

    /// Returns whether the server explicitly reports acknowledged data loss.
    #[must_use]
    pub const fn acknowledged_data_loss(&self) -> bool {
        self.acknowledged_data_loss
    }

    /// Returns the latest revision known to be lost, when it is public.
    #[must_use]
    pub const fn lost_through_revision(&self) -> Option<u64> {
        self.lost_through_revision
    }

    /// Returns the bounded public message, when provided.
    #[must_use]
    pub fn message(&self) -> Option<&DisplaySafeMessage> {
        self.message.as_ref()
    }
}

/// The only state an eventual snapshot installer is permitted to replace.
///
/// Local catalog data, local settings, and credentials remain outside this
/// scope and must be preserved by any transaction that installs a snapshot.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SnapshotInstallScope {
    /// Replace replicated Sync state only.
    ReplicatedStateOnly,
}

impl SnapshotInstallScope {
    /// Returns whether local catalog rows remain outside the replacement scope.
    #[must_use]
    pub const fn preserves_local_catalog(self) -> bool {
        true
    }

    /// Returns whether local settings remain outside the replacement scope.
    #[must_use]
    pub const fn preserves_local_settings(self) -> bool {
        true
    }

    /// Returns whether credentials remain outside the replacement scope.
    #[must_use]
    pub const fn preserves_credentials(self) -> bool {
        true
    }
}

/// A fixed-length SHA-256 digest.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Sha256Digest([u8; Self::LENGTH]);

impl Sha256Digest {
    /// The byte length of a SHA-256 digest.
    pub const LENGTH: usize = 32;

    /// The lowercase hexadecimal length of a SHA-256 digest.
    pub const HEX_LENGTH: usize = Self::LENGTH * 2;

    /// Wraps a digest that has already been decoded from its wire form.
    #[must_use]
    pub const fn from_bytes(value: [u8; Self::LENGTH]) -> Self {
        Self(value)
    }

    /// Decodes an exact lowercase hexadecimal SHA-256 digest.
    ///
    /// # Errors
    ///
    /// Returns [`Sha256DigestError`] when the string is not 64 lowercase
    /// hexadecimal ASCII characters.
    pub fn from_lowercase_hex(value: &str) -> Result<Self, Sha256DigestError> {
        let encoded = value.as_bytes();
        if encoded.len() != Self::HEX_LENGTH {
            return Err(Sha256DigestError::InvalidLength {
                actual: encoded.len(),
                expected: Self::HEX_LENGTH,
            });
        }

        let mut decoded = [0_u8; Self::LENGTH];
        for (index, target) in decoded.iter_mut().enumerate() {
            let offset = index * 2;
            let high = decode_lowercase_hex(encoded[offset], offset)?;
            let low = decode_lowercase_hex(encoded[offset + 1], offset + 1)?;
            *target = (high << 4) | low;
        }

        Ok(Self(decoded))
    }

    /// Calculates the digest of an in-memory byte sequence.
    #[must_use]
    pub fn compute(bytes: &[u8]) -> Self {
        Self(Sha256::digest(bytes).into())
    }

    /// Returns the digest bytes.
    #[must_use]
    pub const fn as_bytes(self) -> [u8; Self::LENGTH] {
        self.0
    }

    /// Encodes the digest as lowercase hexadecimal ASCII.
    #[must_use]
    pub fn to_lowercase_hex(self) -> String {
        const HEX: &[u8; 16] = b"0123456789abcdef";

        let mut encoded = String::with_capacity(Self::HEX_LENGTH);
        for byte in self.0 {
            encoded.push(char::from(HEX[usize::from(byte >> 4)]));
            encoded.push(char::from(HEX[usize::from(byte & 0x0f)]));
        }
        encoded
    }
}

/// An invalid lowercase hexadecimal SHA-256 value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Sha256DigestError {
    /// The encoded value did not have exactly 64 bytes.
    InvalidLength {
        /// The supplied byte count.
        actual: usize,
        /// The required byte count.
        expected: usize,
    },
    /// One byte was not a lowercase hexadecimal ASCII digit.
    InvalidCharacter {
        /// The invalid byte position in the encoded value.
        index: usize,
    },
}

impl fmt::Display for Sha256DigestError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidLength { actual, expected } => {
                write!(
                    formatter,
                    "SHA-256 value has {actual} bytes; expected {expected}"
                )
            }
            Self::InvalidCharacter { index } => {
                write!(
                    formatter,
                    "SHA-256 value has an invalid character at byte {index}"
                )
            }
        }
    }
}

impl Error for Sha256DigestError {}

fn decode_lowercase_hex(value: u8, index: usize) -> Result<u8, Sha256DigestError> {
    match value {
        b'0'..=b'9' => Ok(value - b'0'),
        b'a'..=b'f' => Ok(value - b'a' + 10),
        _ => Err(Sha256DigestError::InvalidCharacter { index }),
    }
}

/// Metadata for one indexed chunk in a snapshot manifest.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChunkDescriptor {
    chunk_index: u32,
    sha256: Sha256Digest,
    byte_length: u32,
}

impl ChunkDescriptor {
    /// Creates one manifest descriptor.
    #[must_use]
    pub const fn new(chunk_index: u32, sha256: Sha256Digest, byte_length: u32) -> Self {
        Self {
            chunk_index,
            sha256,
            byte_length,
        }
    }

    /// Returns this chunk's zero-based index.
    #[must_use]
    pub const fn chunk_index(self) -> u32 {
        self.chunk_index
    }

    /// Returns the expected payload digest.
    #[must_use]
    pub const fn sha256(self) -> Sha256Digest {
        self.sha256
    }

    /// Returns the expected payload length in bytes.
    #[must_use]
    pub const fn byte_length(self) -> u32 {
        self.byte_length
    }
}

/// Immutable bootstrap metadata and the descriptor for every expected chunk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SnapshotManifest {
    snapshot_id: Uuid,
    sync_epoch: Uuid,
    upper_bound_revision: u64,
    schema_version: String,
    total_chunks: u32,
    chunk_size_bytes: u32,
    whole_sha256: Sha256Digest,
    chunks: Vec<ChunkDescriptor>,
}

impl SnapshotManifest {
    /// Creates a manifest value. Use [`validate_manifest`] before accepting it.
    #[must_use]
    pub fn new(
        snapshot_id: Uuid,
        sync_epoch: Uuid,
        upper_bound_revision: u64,
        schema_version: impl Into<String>,
        total_chunks: u32,
        chunk_size_bytes: u32,
        whole_sha256: Sha256Digest,
        chunks: Vec<ChunkDescriptor>,
    ) -> Self {
        Self {
            snapshot_id,
            sync_epoch,
            upper_bound_revision,
            schema_version: schema_version.into(),
            total_chunks,
            chunk_size_bytes,
            whole_sha256,
            chunks,
        }
    }

    /// Returns the snapshot provenance identifier.
    #[must_use]
    pub const fn snapshot_id(&self) -> Uuid {
        self.snapshot_id
    }

    /// Returns the epoch represented by this snapshot.
    #[must_use]
    pub const fn sync_epoch(&self) -> Uuid {
        self.sync_epoch
    }

    /// Returns the last committed revision represented by this snapshot.
    #[must_use]
    pub const fn upper_bound_revision(&self) -> u64 {
        self.upper_bound_revision
    }

    /// Returns the application schema version used to decode this snapshot.
    #[must_use]
    pub fn schema_version(&self) -> &str {
        &self.schema_version
    }

    /// Returns the total number of descriptors and expected chunks.
    #[must_use]
    pub const fn total_chunks(&self) -> u32 {
        self.total_chunks
    }

    /// Returns the maximum payload size permitted for each chunk.
    #[must_use]
    pub const fn chunk_size_bytes(&self) -> u32 {
        self.chunk_size_bytes
    }

    /// Returns the digest of all payloads concatenated by chunk index.
    #[must_use]
    pub const fn whole_sha256(&self) -> Sha256Digest {
        self.whole_sha256
    }

    /// Returns the complete manifest descriptor list.
    #[must_use]
    pub fn chunks(&self) -> &[ChunkDescriptor] {
        &self.chunks
    }
}

/// One downloaded snapshot chunk. It remains untrusted until validation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Chunk {
    snapshot_id: Uuid,
    chunk_index: u32,
    sha256: Sha256Digest,
    payload: Vec<u8>,
}

impl Chunk {
    /// Creates a received chunk with its wire metadata and raw payload.
    #[must_use]
    pub fn new(
        snapshot_id: Uuid,
        chunk_index: u32,
        sha256: Sha256Digest,
        payload: Vec<u8>,
    ) -> Self {
        Self {
            snapshot_id,
            chunk_index,
            sha256,
            payload,
        }
    }

    /// Returns the snapshot provenance identifier supplied with this chunk.
    #[must_use]
    pub const fn snapshot_id(&self) -> Uuid {
        self.snapshot_id
    }

    /// Returns the zero-based chunk index supplied with this chunk.
    #[must_use]
    pub const fn chunk_index(&self) -> u32 {
        self.chunk_index
    }

    /// Returns the digest supplied with this chunk.
    #[must_use]
    pub const fn sha256(&self) -> Sha256Digest {
        self.sha256
    }

    /// Returns the untrusted received payload.
    #[must_use]
    pub fn payload(&self) -> &[u8] {
        &self.payload
    }
}

/// A manifest shape or descriptor inconsistency.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SnapshotManifestError {
    /// The schema version was empty.
    EmptySchemaVersion,
    /// The schema version exceeded [`MAX_SNAPSHOT_SCHEMA_VERSION_CHARS`].
    SchemaVersionTooLong {
        /// The supplied Unicode scalar count.
        actual: usize,
        /// The maximum Unicode scalar count.
        maximum: usize,
    },
    /// The manifest declared no chunks.
    ZeroTotalChunks,
    /// The manifest declared more than [`MAX_SNAPSHOT_CHUNKS`] chunks.
    TotalChunksExceedsMaximum {
        /// The declared chunk count.
        actual: u32,
        /// The maximum accepted chunk count.
        maximum: u32,
    },
    /// The chunk size was zero.
    ZeroChunkSize,
    /// The chunk size exceeded [`MAX_SNAPSHOT_CHUNK_SIZE_BYTES`].
    ChunkSizeExceedsMaximum {
        /// The declared chunk size.
        actual: u32,
        /// The maximum accepted chunk size.
        maximum: u32,
    },
    /// The descriptor count did not match the declared total.
    DescriptorCountMismatch {
        /// The declared count.
        expected: u32,
        /// The actual descriptor count.
        actual: usize,
    },
    /// A descriptor index was outside the declared chunk range.
    DescriptorIndexOutOfRange {
        /// The rejected index.
        chunk_index: u32,
        /// The declared chunk count.
        total_chunks: u32,
    },
    /// More than one descriptor named the same chunk index.
    DuplicateDescriptorIndex {
        /// The repeated chunk index.
        chunk_index: u32,
    },
    /// The descriptor set omitted one required index.
    MissingDescriptorIndex {
        /// The missing zero-based chunk index.
        chunk_index: u32,
    },
    /// A descriptor would allow a payload larger than its manifest chunk size.
    DescriptorLengthExceedsChunkSize {
        /// The affected chunk index.
        chunk_index: u32,
        /// The declared payload length.
        byte_length: u32,
        /// The manifest chunk size.
        chunk_size_bytes: u32,
    },
    /// The declared chunk count cannot be represented on this platform.
    ChunkCountUnsupported {
        /// The declared chunk count.
        total_chunks: u32,
    },
}

impl fmt::Display for SnapshotManifestError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptySchemaVersion => {
                formatter.write_str("snapshot schema version must not be empty")
            }
            Self::SchemaVersionTooLong { actual, maximum } => write!(
                formatter,
                "snapshot schema version has {actual} characters; maximum is {maximum}"
            ),
            Self::ZeroTotalChunks => {
                formatter.write_str("snapshot must contain at least one chunk")
            }
            Self::TotalChunksExceedsMaximum { actual, maximum } => {
                write!(
                    formatter,
                    "snapshot has {actual} chunks; maximum is {maximum}"
                )
            }
            Self::ZeroChunkSize => {
                formatter.write_str("snapshot chunk size must be greater than zero")
            }
            Self::ChunkSizeExceedsMaximum { actual, maximum } => write!(
                formatter,
                "snapshot chunk size is {actual} bytes; maximum is {maximum}"
            ),
            Self::DescriptorCountMismatch { expected, actual } => write!(
                formatter,
                "snapshot has {actual} descriptors; expected {expected}"
            ),
            Self::DescriptorIndexOutOfRange {
                chunk_index,
                total_chunks,
            } => write!(
                formatter,
                "snapshot descriptor index {chunk_index} is outside 0..{total_chunks}"
            ),
            Self::DuplicateDescriptorIndex { chunk_index } => {
                write!(
                    formatter,
                    "snapshot descriptor index {chunk_index} is duplicated"
                )
            }
            Self::MissingDescriptorIndex { chunk_index } => {
                write!(
                    formatter,
                    "snapshot descriptor index {chunk_index} is missing"
                )
            }
            Self::DescriptorLengthExceedsChunkSize {
                chunk_index,
                byte_length,
                chunk_size_bytes,
            } => write!(
                formatter,
                "snapshot descriptor {chunk_index} has {byte_length} bytes; chunk limit is {chunk_size_bytes}"
            ),
            Self::ChunkCountUnsupported { total_chunks } => write!(
                formatter,
                "snapshot chunk count {total_chunks} is unsupported on this platform"
            ),
        }
    }
}

impl Error for SnapshotManifestError {}

/// A received chunk set that cannot safely form the manifest snapshot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SnapshotValidationError {
    /// The manifest was structurally invalid.
    InvalidManifest(SnapshotManifestError),
    /// A chunk named a different snapshot than the manifest.
    ChunkSnapshotIdMismatch {
        /// The index carried by the rejected chunk.
        chunk_index: u32,
    },
    /// A received chunk index was outside the manifest range.
    ChunkIndexOutOfRange {
        /// The rejected chunk index.
        chunk_index: u32,
        /// The manifest chunk count.
        total_chunks: u32,
    },
    /// More than one received chunk named the same index.
    DuplicateChunkIndex {
        /// The repeated chunk index.
        chunk_index: u32,
    },
    /// A required chunk was not received.
    MissingChunk {
        /// The missing zero-based chunk index.
        chunk_index: u32,
    },
    /// A chunk payload length differed from its descriptor.
    ChunkLengthMismatch {
        /// The affected chunk index.
        chunk_index: u32,
        /// The descriptor length in bytes.
        expected: u32,
        /// The received payload length in bytes.
        actual: u64,
    },
    /// The chunk header or manifest descriptor did not match the payload hash.
    ChunkHashMismatch {
        /// The affected chunk index.
        chunk_index: u32,
    },
    /// Concatenated, index-ordered payloads did not match the manifest digest.
    WholeHashMismatch,
}

impl fmt::Display for SnapshotValidationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidManifest(error) => write!(formatter, "invalid snapshot manifest: {error}"),
            Self::ChunkSnapshotIdMismatch { chunk_index } => {
                write!(
                    formatter,
                    "snapshot chunk {chunk_index} has a different snapshot ID"
                )
            }
            Self::ChunkIndexOutOfRange {
                chunk_index,
                total_chunks,
            } => write!(
                formatter,
                "snapshot chunk index {chunk_index} is outside 0..{total_chunks}"
            ),
            Self::DuplicateChunkIndex { chunk_index } => {
                write!(
                    formatter,
                    "snapshot chunk index {chunk_index} is duplicated"
                )
            }
            Self::MissingChunk { chunk_index } => {
                write!(formatter, "snapshot chunk {chunk_index} is missing")
            }
            Self::ChunkLengthMismatch {
                chunk_index,
                expected,
                actual,
            } => write!(
                formatter,
                "snapshot chunk {chunk_index} has {actual} bytes; expected {expected}"
            ),
            Self::ChunkHashMismatch { chunk_index } => {
                write!(
                    formatter,
                    "snapshot chunk {chunk_index} SHA-256 does not match"
                )
            }
            Self::WholeHashMismatch => formatter.write_str("snapshot whole SHA-256 does not match"),
        }
    }
}

impl Error for SnapshotValidationError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::InvalidManifest(error) => Some(error),
            _ => None,
        }
    }
}

/// Validates manifest bounds, descriptor count, contiguous indexes, and sizes.
///
/// The descriptor vector need not be received in index order, but it must name
/// every index from zero through `total_chunks - 1` exactly once.
///
/// # Errors
///
/// Returns [`SnapshotManifestError`] when the manifest cannot safely describe
/// a bounded, complete set of chunks.
pub fn validate_manifest(manifest: &SnapshotManifest) -> Result<(), SnapshotManifestError> {
    let schema_version_length = manifest.schema_version.chars().count();
    if schema_version_length == 0 {
        return Err(SnapshotManifestError::EmptySchemaVersion);
    }
    if schema_version_length > MAX_SNAPSHOT_SCHEMA_VERSION_CHARS {
        return Err(SnapshotManifestError::SchemaVersionTooLong {
            actual: schema_version_length,
            maximum: MAX_SNAPSHOT_SCHEMA_VERSION_CHARS,
        });
    }
    if manifest.total_chunks == 0 {
        return Err(SnapshotManifestError::ZeroTotalChunks);
    }
    if manifest.total_chunks > MAX_SNAPSHOT_CHUNKS {
        return Err(SnapshotManifestError::TotalChunksExceedsMaximum {
            actual: manifest.total_chunks,
            maximum: MAX_SNAPSHOT_CHUNKS,
        });
    }
    if manifest.chunk_size_bytes == 0 {
        return Err(SnapshotManifestError::ZeroChunkSize);
    }
    if manifest.chunk_size_bytes > MAX_SNAPSHOT_CHUNK_SIZE_BYTES {
        return Err(SnapshotManifestError::ChunkSizeExceedsMaximum {
            actual: manifest.chunk_size_bytes,
            maximum: MAX_SNAPSHOT_CHUNK_SIZE_BYTES,
        });
    }
    if manifest.chunks.len() != usize::try_from(manifest.total_chunks).unwrap_or(usize::MAX) {
        return Err(SnapshotManifestError::DescriptorCountMismatch {
            expected: manifest.total_chunks,
            actual: manifest.chunks.len(),
        });
    }

    let chunk_count = snapshot_chunk_count(manifest.total_chunks)?;
    let mut seen = vec![false; chunk_count];
    for descriptor in &manifest.chunks {
        let Some(index) = usize::try_from(descriptor.chunk_index).ok() else {
            return Err(SnapshotManifestError::DescriptorIndexOutOfRange {
                chunk_index: descriptor.chunk_index,
                total_chunks: manifest.total_chunks,
            });
        };
        if index >= chunk_count {
            return Err(SnapshotManifestError::DescriptorIndexOutOfRange {
                chunk_index: descriptor.chunk_index,
                total_chunks: manifest.total_chunks,
            });
        }
        if seen[index] {
            return Err(SnapshotManifestError::DuplicateDescriptorIndex {
                chunk_index: descriptor.chunk_index,
            });
        }
        if descriptor.byte_length > manifest.chunk_size_bytes {
            return Err(SnapshotManifestError::DescriptorLengthExceedsChunkSize {
                chunk_index: descriptor.chunk_index,
                byte_length: descriptor.byte_length,
                chunk_size_bytes: manifest.chunk_size_bytes,
            });
        }
        seen[index] = true;
    }

    for (index, present) in seen.iter().enumerate() {
        if !present {
            return Err(SnapshotManifestError::MissingDescriptorIndex {
                chunk_index: u32::try_from(index).unwrap_or(u32::MAX),
            });
        }
    }

    Ok(())
}

/// Validates a manifest and a complete, unordered set of received chunks.
///
/// This function only validates in-memory values. It does not decode a
/// snapshot, replace state, write a cursor, rebase an outbox, or contact a
/// service. After success, an installer may replace replicated state only;
/// local catalog data, local settings, and credentials remain untouched.
///
/// # Errors
///
/// Returns [`SnapshotValidationError`] for invalid manifest metadata, missing
/// chunks, mismatched lengths, per-chunk hashes, or the whole snapshot hash.
pub fn validate_snapshot(
    manifest: &SnapshotManifest,
    chunks: &[Chunk],
) -> Result<(), SnapshotValidationError> {
    validate_manifest(manifest).map_err(SnapshotValidationError::InvalidManifest)?;

    let chunk_count = snapshot_chunk_count(manifest.total_chunks)
        .map_err(SnapshotValidationError::InvalidManifest)?;
    let mut descriptors = vec![None; chunk_count];
    for descriptor in &manifest.chunks {
        let Some(index) = usize::try_from(descriptor.chunk_index).ok() else {
            return Err(SnapshotValidationError::InvalidManifest(
                SnapshotManifestError::DescriptorIndexOutOfRange {
                    chunk_index: descriptor.chunk_index,
                    total_chunks: manifest.total_chunks,
                },
            ));
        };
        descriptors[index] = Some(descriptor);
    }

    let mut received = vec![None; chunk_count];
    for chunk in chunks {
        if chunk.snapshot_id != manifest.snapshot_id {
            return Err(SnapshotValidationError::ChunkSnapshotIdMismatch {
                chunk_index: chunk.chunk_index,
            });
        }

        let Some(index) = usize::try_from(chunk.chunk_index).ok() else {
            return Err(SnapshotValidationError::ChunkIndexOutOfRange {
                chunk_index: chunk.chunk_index,
                total_chunks: manifest.total_chunks,
            });
        };
        if index >= chunk_count {
            return Err(SnapshotValidationError::ChunkIndexOutOfRange {
                chunk_index: chunk.chunk_index,
                total_chunks: manifest.total_chunks,
            });
        }
        if received[index].is_some() {
            return Err(SnapshotValidationError::DuplicateChunkIndex {
                chunk_index: chunk.chunk_index,
            });
        }

        let Some(descriptor) = descriptors[index] else {
            return Err(SnapshotValidationError::InvalidManifest(
                SnapshotManifestError::MissingDescriptorIndex {
                    chunk_index: chunk.chunk_index,
                },
            ));
        };
        let payload_length = u64::try_from(chunk.payload.len()).unwrap_or(u64::MAX);
        if payload_length != u64::from(descriptor.byte_length) {
            return Err(SnapshotValidationError::ChunkLengthMismatch {
                chunk_index: chunk.chunk_index,
                expected: descriptor.byte_length,
                actual: payload_length,
            });
        }

        let payload_hash = Sha256Digest::compute(&chunk.payload);
        if payload_hash != chunk.sha256 || payload_hash != descriptor.sha256 {
            return Err(SnapshotValidationError::ChunkHashMismatch {
                chunk_index: chunk.chunk_index,
            });
        }
        received[index] = Some(chunk);
    }

    let mut whole_hasher = Sha256::new();
    for (index, chunk) in received.iter().enumerate() {
        let Some(chunk) = chunk else {
            return Err(SnapshotValidationError::MissingChunk {
                chunk_index: u32::try_from(index).unwrap_or(u32::MAX),
            });
        };
        whole_hasher.update(&chunk.payload);
    }
    let whole_hash = Sha256Digest(whole_hasher.finalize().into());
    if whole_hash != manifest.whole_sha256 {
        return Err(SnapshotValidationError::WholeHashMismatch);
    }

    Ok(())
}

fn snapshot_chunk_count(total_chunks: u32) -> Result<usize, SnapshotManifestError> {
    usize::try_from(total_chunks)
        .map_err(|_| SnapshotManifestError::ChunkCountUnsupported { total_chunks })
}

#[cfg(test)]
mod tests {
    use super::{
        Chunk, ChunkDescriptor, RecoveryStatus, Sha256Digest, SnapshotInstallScope,
        SnapshotManifest, SnapshotValidationError, SyncCursor, validate_manifest,
        validate_snapshot,
    };
    use uuid::Uuid;

    fn uuid(value: u128) -> Uuid {
        Uuid::from_u128(value)
    }

    fn fixture() -> (SnapshotManifest, Vec<Chunk>) {
        let snapshot_id = uuid(1);
        let payloads = [
            b"first snapshot payload".as_slice(),
            b"second payload".as_slice(),
        ];
        let descriptors = payloads
            .iter()
            .enumerate()
            .map(|(index, payload)| {
                ChunkDescriptor::new(
                    u32::try_from(index).expect("fixture index fits u32"),
                    Sha256Digest::compute(payload),
                    u32::try_from(payload.len()).expect("fixture length fits u32"),
                )
            })
            .collect::<Vec<_>>();
        let mut whole = Vec::new();
        for payload in payloads {
            whole.extend_from_slice(payload);
        }
        let manifest = SnapshotManifest::new(
            snapshot_id,
            uuid(2),
            42,
            "sync-v2",
            2,
            1_024,
            Sha256Digest::compute(&whole),
            descriptors,
        );
        let chunks = payloads
            .iter()
            .enumerate()
            .map(|(index, payload)| {
                Chunk::new(
                    snapshot_id,
                    u32::try_from(index).expect("fixture index fits u32"),
                    Sha256Digest::compute(payload),
                    payload.to_vec(),
                )
            })
            .collect();

        (manifest, chunks)
    }

    #[test]
    fn validates_a_complete_snapshot_with_unordered_chunks() {
        let (manifest, mut chunks) = fixture();
        chunks.reverse();

        assert_eq!(validate_manifest(&manifest), Ok(()));
        assert_eq!(validate_snapshot(&manifest, &chunks), Ok(()));
    }

    #[test]
    fn rejects_a_missing_chunk() {
        let (manifest, mut chunks) = fixture();
        chunks.pop();

        assert_eq!(
            validate_snapshot(&manifest, &chunks),
            Err(SnapshotValidationError::MissingChunk { chunk_index: 1 })
        );
    }

    #[test]
    fn rejects_a_chunk_with_a_bad_hash() {
        let (manifest, mut chunks) = fixture();
        chunks[1] = Chunk::new(
            chunks[1].snapshot_id(),
            chunks[1].chunk_index(),
            Sha256Digest::compute(b"different hash"),
            chunks[1].payload().to_vec(),
        );

        assert_eq!(
            validate_snapshot(&manifest, &chunks),
            Err(SnapshotValidationError::ChunkHashMismatch { chunk_index: 1 })
        );
    }

    #[test]
    fn rejects_a_snapshot_with_a_bad_whole_hash() {
        let (manifest, chunks) = fixture();
        let manifest = SnapshotManifest::new(
            manifest.snapshot_id(),
            manifest.sync_epoch(),
            manifest.upper_bound_revision(),
            manifest.schema_version(),
            manifest.total_chunks(),
            manifest.chunk_size_bytes(),
            Sha256Digest::compute(b"different whole hash"),
            manifest.chunks().to_vec(),
        );

        assert_eq!(
            validate_snapshot(&manifest, &chunks),
            Err(SnapshotValidationError::WholeHashMismatch)
        );
    }

    #[test]
    fn epoch_mismatch_stops_ordinary_sync_and_preserves_local_data_boundaries() {
        let cursor = SyncCursor::new(uuid(3), 99);

        assert_eq!(cursor.sync_epoch(), uuid(3));
        assert_eq!(cursor.revision(), 99);
        assert!(RecoveryStatus::Normal.allows_ordinary_push_pull());
        assert!(!RecoveryStatus::WriteFrozen.allows_ordinary_push_pull());
        assert_eq!(
            RecoveryStatus::for_epoch_mismatch(),
            RecoveryStatus::RecoveryRequired
        );
        assert!(!RecoveryStatus::for_epoch_mismatch().allows_ordinary_push_pull());
        assert!(SnapshotInstallScope::ReplicatedStateOnly.preserves_local_catalog());
        assert!(SnapshotInstallScope::ReplicatedStateOnly.preserves_local_settings());
        assert!(SnapshotInstallScope::ReplicatedStateOnly.preserves_credentials());
    }
}
