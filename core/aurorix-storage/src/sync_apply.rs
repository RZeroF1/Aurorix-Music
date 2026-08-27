//! Transactional application helpers for the local Sync v2 `SQLite` boundary.
//!
//! The helpers in this module intentionally receive a `rusqlite::Transaction`.
//! A caller can therefore combine local replicated-state writes with the
//! outbox/cursor updates below and commit all of them atomically.  No Cloud,
//! platform, credential, or provider state is represented here.

use std::{error::Error, fmt};

use rusqlite::{OptionalExtension, Transaction, params};
use sha2::{Digest, Sha256};

/// The byte length of a SHA-256 operation digest.
pub const OPERATION_DIGEST_LEN: usize = 32;
const DIGEST_LEN: usize = OPERATION_DIGEST_LEN;

/// Canonical status returned by the Sync v2 writer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CanonicalStatus {
    /// The operation created a new canonical mutation.
    Accepted,
    /// The operation ID and bytes were already committed by the server.
    Duplicate,
    /// The operation was rejected and did not receive a revision.
    Rejected,
}

impl CanonicalStatus {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Accepted => "accepted",
            Self::Duplicate => "duplicate",
            Self::Rejected => "rejected",
        }
    }
}

/// A compact canonical result returned by Sync v2.
///
/// `revision` and `entity_version` are present for accepted and duplicate
/// results and absent for rejected results.  The canonical payload itself is
/// applied by the replicated-state repository; this value carries the
/// ordering and idempotency metadata needed by the local boundary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CanonicalResult {
    operation_id: String,
    entity_id: String,
    status: CanonicalStatus,
    revision: Option<u64>,
    entity_version: Option<u64>,
    error_code: Option<String>,
}

impl CanonicalResult {
    /// Creates an accepted canonical result.
    #[must_use]
    pub fn accepted(
        operation_id: impl Into<String>,
        entity_id: impl Into<String>,
        revision: u64,
        entity_version: u64,
    ) -> Self {
        Self {
            operation_id: operation_id.into(),
            entity_id: entity_id.into(),
            status: CanonicalStatus::Accepted,
            revision: Some(revision),
            entity_version: Some(entity_version),
            error_code: None,
        }
    }

    /// Creates a duplicate canonical result, retaining its original revision.
    #[must_use]
    pub fn duplicate(
        operation_id: impl Into<String>,
        entity_id: impl Into<String>,
        revision: u64,
        entity_version: u64,
    ) -> Self {
        Self {
            operation_id: operation_id.into(),
            entity_id: entity_id.into(),
            status: CanonicalStatus::Duplicate,
            revision: Some(revision),
            entity_version: Some(entity_version),
            error_code: None,
        }
    }

    /// Creates a rejected canonical result with a stable error code.
    #[must_use]
    pub fn rejected(
        operation_id: impl Into<String>,
        entity_id: impl Into<String>,
        error_code: impl Into<String>,
    ) -> Self {
        Self {
            operation_id: operation_id.into(),
            entity_id: entity_id.into(),
            status: CanonicalStatus::Rejected,
            revision: None,
            entity_version: None,
            error_code: Some(error_code.into()),
        }
    }

    /// Returns the operation idempotency key.
    #[must_use]
    pub fn operation_id(&self) -> &str {
        &self.operation_id
    }

    /// Returns the replicated entity id.
    #[must_use]
    pub fn entity_id(&self) -> &str {
        &self.entity_id
    }

    /// Returns the canonical status.
    #[must_use]
    pub const fn status(&self) -> CanonicalStatus {
        self.status
    }

    /// Returns the canonical revision, when one was assigned.
    #[must_use]
    pub const fn revision(&self) -> Option<u64> {
        self.revision
    }

    /// Returns the canonical entity version, when one was assigned.
    #[must_use]
    pub const fn entity_version(&self) -> Option<u64> {
        self.entity_version
    }

    /// Returns the rejection code, when this is a rejected result.
    #[must_use]
    pub fn error_code(&self) -> Option<&str> {
        self.error_code.as_deref()
    }
}

/// The durable local outbox lifecycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutboxState {
    /// The operation has not received a canonical result.
    Pending,
    /// The operation has received an accepted or duplicate result.
    Acknowledged,
    /// The operation is retained only as archive evidence.
    Archived,
}

impl OutboxState {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Acknowledged => "acknowledged",
            Self::Archived => "archived",
        }
    }

    fn parse(value: &str) -> Result<Self, SyncApplyError> {
        match value {
            "pending" => Ok(Self::Pending),
            "acknowledged" => Ok(Self::Acknowledged),
            "archived" => Ok(Self::Archived),
            other => Err(SyncApplyError::CorruptOutboxState {
                operation_id: String::new(),
                state: other.to_owned(),
            }),
        }
    }
}

/// Cursor position in one immutable Sync history.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyncCursor {
    sync_epoch: String,
    revision: u64,
    updated_at: String,
}

impl SyncCursor {
    /// Creates a cursor value for insertion or comparison.
    #[must_use]
    pub fn new(
        sync_epoch: impl Into<String>,
        revision: u64,
        updated_at: impl Into<String>,
    ) -> Self {
        Self {
            sync_epoch: sync_epoch.into(),
            revision,
            updated_at: updated_at.into(),
        }
    }

    /// Returns the history epoch.
    #[must_use]
    pub fn sync_epoch(&self) -> &str {
        &self.sync_epoch
    }

    /// Returns the committed revision.
    #[must_use]
    pub const fn revision(&self) -> u64 {
        self.revision
    }

    /// Returns the timestamp retained when this cursor was advanced.
    #[must_use]
    pub fn updated_at(&self) -> &str {
        &self.updated_at
    }
}

/// Result of applying one canonical result to local Sync state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApplySyncResult {
    cursor: SyncCursor,
    cursor_advanced: bool,
    outbox_state: OutboxState,
    archive_inserted: bool,
}

impl ApplySyncResult {
    /// Returns the cursor after this transaction's writes.
    #[must_use]
    pub fn cursor(&self) -> &SyncCursor {
        &self.cursor
    }

    /// Returns whether this call moved the cursor to a newer revision.
    #[must_use]
    pub const fn cursor_advanced(&self) -> bool {
        self.cursor_advanced
    }

    /// Returns the resulting local outbox state.
    #[must_use]
    pub const fn outbox_state(&self) -> OutboxState {
        self.outbox_state
    }

    /// Returns whether an archive row was inserted (rather than already present).
    #[must_use]
    pub const fn archive_inserted(&self) -> bool {
        self.archive_inserted
    }
}

/// Failures while validating or applying local Sync state.
#[derive(Debug)]
pub enum SyncApplyError {
    /// `SQLite` rejected a statement.
    Sqlite(rusqlite::Error),
    /// The local cursor belongs to another immutable history epoch.
    EpochMismatch { expected: String, actual: String },
    /// Applying a result would move the cursor backwards.
    CursorRegression { current: u64, incoming: u64 },
    /// A canonical result violates the Sync v2 result shape.
    InvalidCanonicalResult(&'static str),
    /// The outbox operation was not found.
    UnknownOperation { operation_id: String },
    /// The result entity differs from the immutable outbox entity.
    EntityMismatch {
        operation_id: String,
        expected: String,
        actual: String,
    },
    /// The supplied digest differs from the retained immutable operation bytes.
    DigestMismatch { operation_id: String },
    /// The stored digest does not match the retained operation bytes.
    CorruptOperationDigest { operation_id: String },
    /// An outbox row contains a state outside the schema contract.
    CorruptOutboxState { operation_id: String, state: String },
    /// A previously archived result conflicts with this result.
    ArchiveConflict { operation_id: String },
    /// An archived operation cannot transition back to acknowledged.
    ArchivedOperation { operation_id: String },
}

impl fmt::Display for SyncApplyError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Sqlite(error) => write!(formatter, "SQLite Sync apply failed: {error}"),
            Self::EpochMismatch { expected, actual } => {
                write!(
                    formatter,
                    "Sync epoch mismatch: expected {expected:?}, found {actual:?}"
                )
            }
            Self::CursorRegression { current, incoming } => {
                write!(
                    formatter,
                    "Sync cursor regression: current {current}, incoming {incoming}"
                )
            }
            Self::InvalidCanonicalResult(message) => {
                write!(formatter, "invalid canonical result: {message}")
            }
            Self::UnknownOperation { operation_id } => {
                write!(formatter, "unknown outbox operation {operation_id:?}")
            }
            Self::EntityMismatch {
                operation_id,
                expected,
                actual,
            } => write!(
                formatter,
                "operation {operation_id:?} entity mismatch: expected {expected:?}, found {actual:?}"
            ),
            Self::DigestMismatch { operation_id } => {
                write!(
                    formatter,
                    "operation {operation_id:?} digest does not match supplied bytes"
                )
            }
            Self::CorruptOperationDigest { operation_id } => {
                write!(
                    formatter,
                    "operation {operation_id:?} has a corrupt stored digest"
                )
            }
            Self::CorruptOutboxState {
                operation_id,
                state,
            } => {
                write!(
                    formatter,
                    "operation {operation_id:?} has invalid outbox state {state:?}"
                )
            }
            Self::ArchiveConflict { operation_id } => {
                write!(
                    formatter,
                    "archive row for operation {operation_id:?} conflicts"
                )
            }
            Self::ArchivedOperation { operation_id } => {
                write!(
                    formatter,
                    "archived operation {operation_id:?} cannot be acknowledged"
                )
            }
        }
    }
}

impl Error for SyncApplyError {}

impl From<rusqlite::Error> for SyncApplyError {
    fn from(error: rusqlite::Error) -> Self {
        Self::Sqlite(error)
    }
}

/// Inserts an outbox operation while retaining exact bytes and their SHA-256 digest.
///
/// Repeating the same operation ID and bytes is an idempotent no-op. Reusing an
/// ID with different bytes returns [`SyncApplyError::DigestMismatch`].
///
/// # Errors
///
/// Returns [`SyncApplyError`] when the existing operation conflicts, its state
/// is invalid, or `SQLite` rejects the insert.
pub fn enqueue_outbox(
    transaction: &Transaction<'_>,
    operation_id: &str,
    entity_id: &str,
    operation_bytes: &[u8],
    base_entity_version: Option<u64>,
    created_at: &str,
) -> Result<OutboxState, SyncApplyError> {
    let digest = digest(operation_bytes);
    let base_entity_version = base_entity_version
        .map(|value| {
            i64::try_from(value).map_err(|_| {
                SyncApplyError::InvalidCanonicalResult("entity version exceeds SQLite range")
            })
        })
        .transpose()?;
    let existing: Option<(String, String, Vec<u8>, Vec<u8>)> = transaction
        .query_row(
            "SELECT entity_id, state, operation_bytes, operation_digest
             FROM sync_outbox WHERE operation_id = ?1",
            [operation_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .optional()?;
    if let Some((retained_entity, state, retained_bytes, retained_digest)) = existing {
        if retained_entity != entity_id
            || retained_bytes != operation_bytes
            || retained_digest != digest
        {
            return Err(SyncApplyError::DigestMismatch {
                operation_id: operation_id.to_owned(),
            });
        }
        let state = OutboxState::parse(&state).map_err(|error| match error {
            SyncApplyError::CorruptOutboxState { state, .. } => {
                SyncApplyError::CorruptOutboxState {
                    operation_id: operation_id.to_owned(),
                    state,
                }
            }
            other => other,
        })?;
        return Ok(state);
    }

    transaction.execute(
        "INSERT INTO sync_outbox
            (operation_id, entity_id, operation_bytes, operation_digest,
             base_entity_version, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![
            operation_id,
            entity_id,
            operation_bytes,
            digest.as_slice(),
            base_entity_version,
            created_at
        ],
    )?;
    Ok(OutboxState::Pending)
}

/// Reads the singleton local cursor, if it has been initialized.
///
/// # Errors
///
/// Returns [`SyncApplyError::Sqlite`] when the cursor cannot be read or its
/// revision cannot be represented as an unsigned integer.
pub fn read_cursor(transaction: &Transaction<'_>) -> Result<Option<SyncCursor>, SyncApplyError> {
    transaction
        .query_row(
            "SELECT sync_epoch, revision, updated_at
             FROM sync_replicated_cursor WHERE id = 1",
            [],
            |row| {
                let revision: i64 = row.get(1)?;
                Ok((row.get::<_, String>(0)?, revision, row.get::<_, String>(2)?))
            },
        )
        .optional()?
        .map(|(epoch, revision, updated_at)| {
            u64::try_from(revision)
                .map(|revision| SyncCursor::new(epoch, revision, updated_at))
                .map_err(|_| {
                    SyncApplyError::Sqlite(rusqlite::Error::IntegralValueOutOfRange(1, revision))
                })
        })
        .transpose()
}

/// Applies one canonical result, outbox transition, archive row, and cursor update.
///
/// All writes execute on the supplied transaction. The cursor is initialized at
/// revision zero when first needed, advances only to a strictly newer revision,
/// and rejects an epoch mismatch before mutating any row. Accepted and duplicate
/// results acknowledge the outbox operation; rejected results archive it without
/// advancing the cursor. Repeating an identical result is idempotent.
///
/// # Errors
///
/// Returns [`SyncApplyError`] when the result shape, epoch, cursor ordering,
/// outbox identity, digest, or archive consistency check fails.
#[allow(clippy::too_many_lines)]
pub fn apply_canonical_result(
    transaction: &Transaction<'_>,
    sync_epoch: &str,
    result: &CanonicalResult,
    archived_at: &str,
    updated_at: &str,
) -> Result<ApplySyncResult, SyncApplyError> {
    validate_result(result)?;
    let operation_id = result.operation_id();
    let (entity_id, state_raw, operation_bytes, operation_digest): (
        String,
        String,
        Vec<u8>,
        Vec<u8>,
    ) = transaction
        .query_row(
            "SELECT entity_id, state, operation_bytes, operation_digest
                 FROM sync_outbox WHERE operation_id = ?1",
            [operation_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .optional()?
        .ok_or_else(|| SyncApplyError::UnknownOperation {
            operation_id: operation_id.to_owned(),
        })?;
    if entity_id != result.entity_id() {
        return Err(SyncApplyError::EntityMismatch {
            operation_id: operation_id.to_owned(),
            expected: entity_id,
            actual: result.entity_id().to_owned(),
        });
    }
    let expected_digest = digest(&operation_bytes);
    if operation_digest.as_slice() != expected_digest {
        return Err(SyncApplyError::CorruptOperationDigest {
            operation_id: operation_id.to_owned(),
        });
    }
    let state = OutboxState::parse(&state_raw).map_err(|error| match error {
        SyncApplyError::CorruptOutboxState { state, .. } => SyncApplyError::CorruptOutboxState {
            operation_id: operation_id.to_owned(),
            state,
        },
        other => other,
    })?;

    let current = read_cursor(transaction)?;
    let current = match current {
        Some(cursor) => {
            if cursor.sync_epoch() != sync_epoch {
                return Err(SyncApplyError::EpochMismatch {
                    expected: sync_epoch.to_owned(),
                    actual: cursor.sync_epoch().to_owned(),
                });
            }
            cursor
        }
        None => SyncCursor::new(sync_epoch, 0, updated_at),
    };
    let incoming_revision = result.revision().unwrap_or(current.revision());
    if incoming_revision < current.revision() {
        return Err(SyncApplyError::CursorRegression {
            current: current.revision(),
            incoming: incoming_revision,
        });
    }

    let next_state = match result.status() {
        CanonicalStatus::Accepted | CanonicalStatus::Duplicate => match state {
            OutboxState::Pending | OutboxState::Acknowledged => OutboxState::Acknowledged,
            OutboxState::Archived => {
                return Err(SyncApplyError::ArchivedOperation {
                    operation_id: operation_id.to_owned(),
                });
            }
        },
        CanonicalStatus::Rejected => match state {
            OutboxState::Pending | OutboxState::Acknowledged | OutboxState::Archived => {
                OutboxState::Archived
            }
        },
    };
    let canonical_revision = result.revision().map(sqlite_u64).transpose()?;
    let canonical_entity_version = result.entity_version().map(sqlite_u64).transpose()?;
    let acknowledged_at = matches!(
        result.status(),
        CanonicalStatus::Accepted | CanonicalStatus::Duplicate
    )
    .then_some(updated_at);
    transaction.execute(
        "UPDATE sync_outbox SET state = ?2, acknowledged_at = COALESCE(?3, acknowledged_at),
             canonical_revision = ?4, canonical_entity_version = ?5
         WHERE operation_id = ?1",
        params![
            operation_id,
            next_state.as_str(),
            acknowledged_at,
            canonical_revision,
            canonical_entity_version
        ],
    )?;

    let archive_inserted = insert_archive(
        transaction,
        operation_id,
        result.entity_id(),
        &operation_bytes,
        &operation_digest,
        result.status().as_str(),
        canonical_revision,
        archived_at,
    )?;

    let cursor_advanced = incoming_revision > current.revision();
    let next_cursor = if cursor_advanced || read_cursor(transaction)?.is_none() {
        transaction.execute(
            "INSERT INTO sync_replicated_cursor (id, sync_epoch, revision, updated_at)
             VALUES (1, ?1, ?2, ?3)
             ON CONFLICT(id) DO UPDATE SET sync_epoch = excluded.sync_epoch,
                 revision = excluded.revision, updated_at = excluded.updated_at",
            params![sync_epoch, sqlite_u64(incoming_revision)?, updated_at],
        )?;
        SyncCursor::new(sync_epoch, incoming_revision, updated_at)
    } else {
        current
    };

    Ok(ApplySyncResult {
        cursor: next_cursor,
        cursor_advanced,
        outbox_state: next_state,
        archive_inserted,
    })
}

fn validate_result(result: &CanonicalResult) -> Result<(), SyncApplyError> {
    if result.operation_id.trim().is_empty() || result.entity_id.trim().is_empty() {
        return Err(SyncApplyError::InvalidCanonicalResult(
            "operation and entity IDs must not be empty",
        ));
    }
    match result.status() {
        CanonicalStatus::Accepted | CanonicalStatus::Duplicate => {
            if result.revision().is_none() || result.revision() == Some(0) {
                return Err(SyncApplyError::InvalidCanonicalResult(
                    "accepted/duplicate result requires a positive revision",
                ));
            }
            if result.entity_version().is_none() {
                return Err(SyncApplyError::InvalidCanonicalResult(
                    "accepted/duplicate result requires entity version",
                ));
            }
            if result.error_code().is_some() {
                return Err(SyncApplyError::InvalidCanonicalResult(
                    "accepted/duplicate result must not carry an error code",
                ));
            }
        }
        CanonicalStatus::Rejected => {
            if result.revision().is_some() || result.entity_version().is_some() {
                return Err(SyncApplyError::InvalidCanonicalResult(
                    "rejected result must not carry revision or entity version",
                ));
            }
            if result
                .error_code()
                .is_none_or(|code| code.trim().is_empty())
            {
                return Err(SyncApplyError::InvalidCanonicalResult(
                    "rejected result requires an error code",
                ));
            }
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn insert_archive(
    transaction: &Transaction<'_>,
    operation_id: &str,
    entity_id: &str,
    operation_bytes: &[u8],
    operation_digest: &[u8],
    outcome: &str,
    canonical_revision: Option<i64>,
    archived_at: &str,
) -> Result<bool, SyncApplyError> {
    let existing: Option<(String, Vec<u8>, Vec<u8>, String, Option<i64>)> = transaction
        .query_row(
            "SELECT entity_id, operation_bytes, operation_digest, outcome, canonical_revision
             FROM sync_operation_archive WHERE operation_id = ?1",
            [operation_id],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                ))
            },
        )
        .optional()?;
    if let Some((
        existing_entity,
        existing_bytes,
        existing_digest,
        existing_outcome,
        existing_revision,
    )) = existing
    {
        let same_outcome = existing_outcome == outcome
            || matches!(
                (existing_outcome.as_str(), outcome),
                ("accepted", "duplicate") | ("duplicate", "accepted")
            );
        if existing_entity != entity_id
            || existing_bytes != operation_bytes
            || existing_digest != operation_digest
            || !same_outcome
            || existing_revision != canonical_revision
        {
            return Err(SyncApplyError::ArchiveConflict {
                operation_id: operation_id.to_owned(),
            });
        }
        return Ok(false);
    }
    transaction.execute(
        "INSERT INTO sync_operation_archive
            (operation_id, entity_id, operation_bytes, operation_digest, outcome,
             canonical_revision, archived_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            operation_id,
            entity_id,
            operation_bytes,
            operation_digest,
            outcome,
            canonical_revision,
            archived_at
        ],
    )?;
    Ok(true)
}

fn sqlite_u64(value: u64) -> Result<i64, SyncApplyError> {
    i64::try_from(value).map_err(|_| {
        SyncApplyError::InvalidCanonicalResult("revision or version exceeds SQLite integer range")
    })
}

fn digest(bytes: &[u8]) -> [u8; DIGEST_LEN] {
    Sha256::digest(bytes).into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{migration::apply_migrations, sync_schema::SYNC_MIGRATIONS};
    use rusqlite::Connection;

    fn database() -> Connection {
        let mut connection = Connection::open_in_memory().expect("in-memory SQLite");
        apply_migrations(&mut connection, SYNC_MIGRATIONS).expect("Sync migration applies");
        connection
    }

    fn enqueue(connection: &mut Connection) {
        let transaction = connection.transaction().expect("transaction");
        assert_eq!(
            super::enqueue_outbox(&transaction, "op-1", "entity-1", b"bytes", None, "t0")
                .expect("enqueue"),
            OutboxState::Pending
        );
        transaction.commit().expect("commit");
    }

    #[test]
    fn accepted_result_advances_cursor_acknowledges_and_archives_atomically() {
        let mut connection = database();
        enqueue(&mut connection);
        let transaction = connection.transaction().expect("transaction");
        let applied = apply_canonical_result(
            &transaction,
            "epoch-a",
            &CanonicalResult::accepted("op-1", "entity-1", 8, 3),
            "t2",
            "t1",
        )
        .expect("apply result");
        assert_eq!(applied.cursor(), &SyncCursor::new("epoch-a", 8, "t1"));
        assert!(applied.cursor_advanced());
        assert_eq!(applied.outbox_state(), OutboxState::Acknowledged);
        assert!(applied.archive_inserted());
        transaction.commit().expect("commit");
        let row: (String, i64, String, String, Option<i64>) = connection
            .query_row(
                "SELECT c.sync_epoch, c.revision, o.state, a.outcome, a.canonical_revision
                 FROM sync_replicated_cursor c
                 JOIN sync_outbox o ON o.operation_id = 'op-1'
                 JOIN sync_operation_archive a ON a.operation_id = 'op-1'",
                [],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                    ))
                },
            )
            .expect("durable rows");
        assert_eq!(
            row,
            (
                "epoch-a".into(),
                8,
                "acknowledged".into(),
                "accepted".into(),
                Some(8)
            )
        );
    }

    #[test]
    fn duplicate_retry_is_idempotent_and_does_not_regress_cursor() {
        let mut connection = database();
        enqueue(&mut connection);
        {
            let transaction = connection.transaction().expect("transaction");
            apply_canonical_result(
                &transaction,
                "epoch-a",
                &CanonicalResult::accepted("op-1", "entity-1", 8, 3),
                "t2",
                "t1",
            )
            .expect("first apply");
            transaction.commit().expect("commit");
        }
        let transaction = connection.transaction().expect("transaction");
        let applied = apply_canonical_result(
            &transaction,
            "epoch-a",
            &CanonicalResult::duplicate("op-1", "entity-1", 8, 3),
            "t3",
            "t4",
        )
        .expect("duplicate apply");
        assert!(!applied.cursor_advanced());
        assert!(!applied.archive_inserted());
        transaction.commit().expect("commit");
        let cursor = connection
            .query_row(
                "SELECT revision, updated_at FROM sync_replicated_cursor",
                [],
                |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)),
            )
            .expect("cursor");
        assert_eq!(cursor, (8, "t1".to_owned()));
    }

    #[test]
    fn epoch_mismatch_and_regression_are_rejected_without_writes() {
        let mut connection = database();
        enqueue(&mut connection);
        {
            let transaction = connection.transaction().expect("transaction");
            apply_canonical_result(
                &transaction,
                "epoch-a",
                &CanonicalResult::accepted("op-1", "entity-1", 8, 3),
                "t2",
                "t1",
            )
            .expect("first apply");
            transaction.commit().expect("commit");
        }
        let transaction = connection.transaction().expect("transaction");
        assert!(matches!(
            apply_canonical_result(
                &transaction,
                "epoch-b",
                &CanonicalResult::duplicate("op-1", "entity-1", 9, 3),
                "t3",
                "t3",
            ),
            Err(SyncApplyError::EpochMismatch { .. })
        ));
        assert!(matches!(
            apply_canonical_result(
                &transaction,
                "epoch-a",
                &CanonicalResult::duplicate("op-1", "entity-1", 7, 3),
                "t3",
                "t3",
            ),
            Err(SyncApplyError::CursorRegression {
                current: 8,
                incoming: 7
            })
        ));
        transaction.rollback().expect("rollback");
        let state: String = connection
            .query_row(
                "SELECT state FROM sync_outbox WHERE operation_id = 'op-1'",
                [],
                |row| row.get(0),
            )
            .expect("outbox state");
        assert_eq!(state, "acknowledged");
    }

    #[test]
    fn rejected_result_archives_without_advancing_and_retry_is_stable() {
        let mut connection = database();
        enqueue(&mut connection);
        let transaction = connection.transaction().expect("transaction");
        let result = CanonicalResult::rejected("op-1", "entity-1", "entity_deleted");
        let applied = apply_canonical_result(&transaction, "epoch-a", &result, "t2", "t1")
            .expect("rejected apply");
        assert_eq!(applied.cursor().revision(), 0);
        assert!(!applied.cursor_advanced());
        assert_eq!(applied.outbox_state(), OutboxState::Archived);
        transaction.commit().expect("commit");
        let transaction = connection.transaction().expect("transaction");
        let repeated = apply_canonical_result(&transaction, "epoch-a", &result, "t3", "t3")
            .expect("rejected retry");
        assert!(!repeated.archive_inserted());
        transaction.commit().expect("commit");
        let row: (i64, String, String) = connection
            .query_row(
                "SELECT c.revision, o.state, a.outcome
                 FROM sync_replicated_cursor c
                 JOIN sync_outbox o ON o.operation_id = 'op-1'
                 JOIN sync_operation_archive a ON a.operation_id = 'op-1'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .expect("durable rejected rows");
        assert_eq!(row, (0, "archived".into(), "rejected".into()));
    }
}
