//! Transaction helpers for persisting local catalog scan observations.
//!
//! The helpers operate only on the existing `local_catalog_scan` and
//! `local_catalog_asset` tables. They do not discover files, interpret
//! locators, or make an asset observation a replicated operation. Callers
//! should invoke them from one bounded write transaction.

use rusqlite::{Transaction, params};

/// A stable identifier for one local catalog scan run.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ScanId(String);

impl ScanId {
    /// Creates a scan identifier from a non-empty value.
    ///
    /// # Errors
    ///
    /// Returns [`ScanIdError::Empty`] when the value is empty after trimming.
    pub fn new(value: impl AsRef<str>) -> Result<Self, ScanIdError> {
        let value = value.as_ref().trim();
        if value.is_empty() {
            return Err(ScanIdError::Empty);
        }
        Ok(Self(value.to_owned()))
    }

    /// Returns the identifier value used by `SQLite`.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for ScanId {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

/// Invalid scan identifier input.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScanIdError {
    /// The identifier contained no non-whitespace characters.
    Empty,
}

impl std::fmt::Display for ScanIdError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("scan ID must not be empty")
    }
}

impl std::error::Error for ScanIdError {}

/// The terminal result recorded when a scan run finishes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScanResult {
    /// Every operation in the scan completed successfully.
    Completed,
    /// The scan stopped with one or more errors.
    Failed,
}

impl ScanResult {
    /// Returns the persisted state value.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Completed => "completed",
            Self::Failed => "failed",
        }
    }
}

/// The persisted state of one local asset after an observation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AssetState {
    /// A locator was found but has not been probed.
    Discovered,
    /// Metadata probing is in progress.
    Probing,
    /// The asset is readable and usable.
    Active,
    /// A previously active asset changed.
    Changed,
    /// The locator was not found during a scan.
    Missing,
    /// Access was denied.
    PermissionDenied,
    /// The asset format is unsupported.
    Unsupported,
    /// Probing failed for an unspecified reason.
    Error,
    /// A possible replacement was found for a missing asset.
    RelinkCandidate,
    /// The asset was explicitly retired.
    Tombstoned,
}

impl AssetState {
    /// Returns the value accepted by the catalog state constraint.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Discovered => "discovered",
            Self::Probing => "probing",
            Self::Active => "active",
            Self::Changed => "changed",
            Self::Missing => "missing",
            Self::PermissionDenied => "permission_denied",
            Self::Unsupported => "unsupported",
            Self::Error => "error",
            Self::RelinkCandidate => "relink_candidate",
            Self::Tombstoned => "tombstoned",
        }
    }
}

/// Starts a scan run in the `running` state.
///
/// The timestamp is an opaque caller-provided Unix-time value. Keeping time
/// acquisition outside storage makes tests and platform scanners deterministic.
///
/// # Errors
///
/// Returns the underlying `SQLite` error when the scan ID is already present or
/// the catalog schema rejects the insert.
pub fn start_scan(
    transaction: &Transaction<'_>,
    scan_id: &ScanId,
    started_at: i64,
) -> rusqlite::Result<usize> {
    transaction.execute(
        "INSERT INTO local_catalog_scan (id, started_at, state) VALUES (?1, ?2, 'running')",
        params![scan_id.as_str(), started_at],
    )
}

/// Finishes a running scan and records its terminal result and timestamp.
///
/// A result can only transition a row that is still `running`; this prevents a
/// late completion from overwriting a previously recorded terminal state.
///
/// # Errors
///
/// Returns the underlying `SQLite` error when the update cannot be executed.
pub fn finish_scan(
    transaction: &Transaction<'_>,
    scan_id: &ScanId,
    finished_at: i64,
    result: ScanResult,
) -> rusqlite::Result<usize> {
    transaction.execute(
        "UPDATE local_catalog_scan
         SET finished_at = ?2, state = ?3
         WHERE id = ?1 AND state = 'running'",
        params![scan_id.as_str(), finished_at, result.as_str()],
    )
}

/// Records that an asset was seen by a running scan.
///
/// The update is deliberately scoped by the scan's `running` row. A caller
/// cannot attach a new observation to an unknown or already finished scan.
///
/// # Errors
///
/// Returns the underlying `SQLite` error when the update cannot be executed.
pub fn mark_asset_seen(
    transaction: &Transaction<'_>,
    scan_id: &ScanId,
    asset_id: &str,
    state: AssetState,
) -> rusqlite::Result<usize> {
    transaction.execute(
        "UPDATE local_catalog_asset
         SET last_seen_scan_id = ?1, state = ?3
         WHERE id = ?2
           AND EXISTS (
               SELECT 1 FROM local_catalog_scan
               WHERE id = ?1 AND state = 'running'
           )",
        params![scan_id.as_str(), asset_id, state.as_str()],
    )
}

/// Marks assets absent from a running scan as `missing`.
///
/// `directory_id` limits the operation to one configured catalog directory.
/// When it is `None`, all local assets are considered in scope. An asset seen
/// by a newer scan (ordered by `(started_at, id)`) is left untouched, so an
/// older scan finishing late cannot erase a newer observation.
///
/// # Errors
///
/// Returns the underlying `SQLite` error when the update cannot be executed.
pub fn mark_missing_assets(
    transaction: &Transaction<'_>,
    scan_id: &ScanId,
    directory_id: Option<&str>,
) -> rusqlite::Result<usize> {
    transaction.execute(
        "UPDATE local_catalog_asset
         SET state = 'missing'
         WHERE (?2 IS NULL OR directory_id = ?2)
           AND state <> 'tombstoned'
           AND EXISTS (
               SELECT 1 FROM local_catalog_scan current_scan
               WHERE current_scan.id = ?1 AND current_scan.state = 'running'
           )
           AND (
               last_seen_scan_id IS NULL
               OR EXISTS (
                   SELECT 1
                   FROM local_catalog_scan seen_scan
                   JOIN local_catalog_scan current_scan ON current_scan.id = ?1
                   WHERE seen_scan.id = local_catalog_asset.last_seen_scan_id
                     AND (
                         seen_scan.started_at < current_scan.started_at
                         OR (
                             seen_scan.started_at = current_scan.started_at
                             AND seen_scan.id < current_scan.id
                         )
                     )
               )
           )",
        params![scan_id.as_str(), directory_id],
    )
}

#[cfg(test)]
mod tests {
    use rusqlite::{Connection, Transaction};

    use super::{
        AssetState, ScanId, ScanResult, finish_scan, mark_asset_seen, mark_missing_assets,
        start_scan,
    };
    use crate::{catalog_schema::LOCAL_CATALOG_MIGRATIONS, migration::apply_migrations};

    fn database() -> Connection {
        let mut connection = Connection::open_in_memory().expect("in-memory SQLite");
        connection
            .execute_batch("PRAGMA foreign_keys = ON")
            .expect("enable foreign keys");
        apply_migrations(&mut connection, LOCAL_CATALOG_MIGRATIONS)
            .expect("catalog migration applies");
        connection
    }

    fn id(value: &str) -> ScanId {
        ScanId::new(value).expect("valid scan ID")
    }

    fn insert_asset(transaction: &Transaction<'_>, asset_id: &str, directory_id: &str) {
        transaction
            .execute(
                "INSERT INTO local_catalog_directory (id, locator, platform)
                 VALUES (?1, ?2, 'test')",
                rusqlite::params![directory_id, format!("file:///{directory_id}")],
            )
            .expect("directory insert");
        transaction
            .execute(
                "INSERT INTO local_catalog_asset (id, directory_id, locator, state)
                 VALUES (?1, ?2, ?3, 'active')",
                rusqlite::params![asset_id, directory_id, format!("file:///{asset_id}.flac")],
            )
            .expect("asset insert");
    }

    #[test]
    fn successful_scan_records_seen_state_and_completion() {
        let mut connection = database();
        let scan = id("scan-success");
        let transaction = connection.transaction().expect("transaction");
        insert_asset(&transaction, "asset-1", "directory-1");
        assert_eq!(start_scan(&transaction, &scan, 100), Ok(1));
        assert_eq!(
            mark_asset_seen(&transaction, &scan, "asset-1", AssetState::Active),
            Ok(1)
        );
        assert_eq!(
            finish_scan(&transaction, &scan, 110, ScanResult::Completed),
            Ok(1)
        );
        transaction.commit().expect("commit");

        let row: (String, String, i64, Option<i64>) = connection
            .query_row(
                "SELECT local_catalog_asset.state, local_catalog_asset.last_seen_scan_id,
                        local_catalog_scan.started_at, local_catalog_scan.finished_at
                 FROM local_catalog_asset
                 JOIN local_catalog_scan ON local_catalog_scan.id = local_catalog_asset.last_seen_scan_id
                 WHERE local_catalog_asset.id = 'asset-1'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .expect("scan row");
        assert_eq!(
            row,
            (
                "active".to_owned(),
                "scan-success".to_owned(),
                100,
                Some(110)
            )
        );
    }

    #[test]
    fn failed_scan_is_terminal_and_cannot_receive_late_observations() {
        let mut connection = database();
        let transaction = connection.transaction().expect("transaction");
        insert_asset(&transaction, "asset-1", "directory-1");
        let scan = id("scan-failed");
        assert_eq!(start_scan(&transaction, &scan, 200), Ok(1));
        assert_eq!(
            finish_scan(&transaction, &scan, 210, ScanResult::Failed),
            Ok(1)
        );
        assert_eq!(
            mark_asset_seen(&transaction, &scan, "asset-1", AssetState::Active),
            Ok(0)
        );
        assert_eq!(
            finish_scan(&transaction, &scan, 220, ScanResult::Completed),
            Ok(0)
        );
        transaction.commit().expect("commit");

        let state: String = connection
            .query_row(
                "SELECT state FROM local_catalog_scan WHERE id = 'scan-failed'",
                [],
                |row| row.get(0),
            )
            .expect("scan state");
        assert_eq!(state, "failed");
    }

    #[test]
    fn older_scan_cannot_mark_asset_seen_by_newer_scan_as_missing() {
        let mut connection = database();
        let transaction = connection.transaction().expect("transaction");
        insert_asset(&transaction, "asset-1", "directory-1");
        let old_scan = id("scan-old");
        let new_scan = id("scan-new");
        assert_eq!(start_scan(&transaction, &old_scan, 300), Ok(1));
        assert_eq!(start_scan(&transaction, &new_scan, 400), Ok(1));
        assert_eq!(
            mark_asset_seen(&transaction, &new_scan, "asset-1", AssetState::Active),
            Ok(1)
        );
        assert_eq!(
            mark_missing_assets(&transaction, &old_scan, Some("directory-1")),
            Ok(0)
        );
        transaction.commit().expect("commit");

        let state: String = connection
            .query_row(
                "SELECT state FROM local_catalog_asset WHERE id = 'asset-1'",
                [],
                |row| row.get(0),
            )
            .expect("asset state");
        assert_eq!(state, "active");
    }

    #[test]
    fn current_scan_marks_only_unseen_assets_in_requested_directory() {
        let mut connection = database();
        let transaction = connection.transaction().expect("transaction");
        insert_asset(&transaction, "asset-1", "directory-1");
        insert_asset(&transaction, "asset-2", "directory-2");
        let scan = id("scan-missing");
        assert_eq!(start_scan(&transaction, &scan, 500), Ok(1));
        assert_eq!(
            mark_asset_seen(&transaction, &scan, "asset-2", AssetState::Active),
            Ok(1)
        );
        assert_eq!(
            mark_missing_assets(&transaction, &scan, Some("directory-1")),
            Ok(1)
        );
        transaction.commit().expect("commit");

        let states: Vec<(String, String)> = connection
            .prepare("SELECT id, state FROM local_catalog_asset ORDER BY id")
            .expect("prepare")
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
            .expect("query")
            .collect::<rusqlite::Result<_>>()
            .expect("rows");
        assert_eq!(
            states,
            vec![
                ("asset-1".to_owned(), "missing".to_owned()),
                ("asset-2".to_owned(), "active".to_owned()),
            ]
        );
    }
}
