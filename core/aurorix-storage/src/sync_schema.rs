//! `SQLite` schema for the device-local Sync state boundary.
//!
//! This migration stores only the durable facts needed to resume Sync and to
//! retain local evidence while a later transport/reducer layer is offline.
//! It intentionally has no Cloud, platform, credential, or provider columns.
//! Operation bytes and their digest are retained verbatim so an operation ID
//! remains an idempotency key across retries and rebase.

use crate::migration::Migration;

/// SQL for the append-only Sync schema migration (version 3).
///
/// The cursor is a single row (`id = 1`) for the replicated history known by
/// this device. Outbox rows are mutable only in their local lifecycle state;
/// operation bytes and digest are required to remain present for retries and
/// diagnostics. Archive rows and tombstones are keyed by their replicated IDs,
/// while conflict-vector entries are normalized by actor/device and counter.
pub const SYNC_SCHEMA_SQL: &str = r"
CREATE TABLE sync_replicated_cursor (
    id INTEGER PRIMARY KEY NOT NULL CHECK (id = 1),
    sync_epoch TEXT NOT NULL CHECK (length(trim(sync_epoch)) > 0),
    revision INTEGER NOT NULL CHECK (revision >= 0),
    updated_at TEXT NOT NULL CHECK (length(trim(updated_at)) > 0)
);

CREATE TABLE sync_outbox (
    operation_id TEXT PRIMARY KEY NOT NULL CHECK (length(trim(operation_id)) > 0),
    entity_id TEXT NOT NULL CHECK (length(trim(entity_id)) > 0),
    operation_bytes BLOB NOT NULL,
    operation_digest BLOB NOT NULL CHECK (length(operation_digest) = 32),
    base_entity_version INTEGER CHECK (
        base_entity_version IS NULL OR base_entity_version >= 0
    ),
    state TEXT NOT NULL DEFAULT 'pending' CHECK (
        state IN ('pending', 'acknowledged', 'archived')
    ),
    created_at TEXT NOT NULL CHECK (length(trim(created_at)) > 0),
    acknowledged_at TEXT,
    canonical_revision INTEGER CHECK (
        canonical_revision IS NULL OR canonical_revision >= 0
    ),
    canonical_entity_version INTEGER CHECK (
        canonical_entity_version IS NULL OR canonical_entity_version >= 0
    )
);

CREATE INDEX sync_outbox_pending_idx
    ON sync_outbox(state, created_at, operation_id);

CREATE TABLE sync_operation_archive (
    operation_id TEXT PRIMARY KEY NOT NULL CHECK (length(trim(operation_id)) > 0),
    entity_id TEXT NOT NULL CHECK (length(trim(entity_id)) > 0),
    operation_bytes BLOB NOT NULL,
    operation_digest BLOB NOT NULL CHECK (length(operation_digest) = 32),
    outcome TEXT NOT NULL CHECK (
        outcome IN ('accepted', 'duplicate', 'rejected', 'replayed', 'needs_user_review')
    ),
    canonical_revision INTEGER CHECK (
        canonical_revision IS NULL OR canonical_revision >= 0
    ),
    archived_at TEXT NOT NULL CHECK (length(trim(archived_at)) > 0)
);

CREATE INDEX sync_operation_archive_entity_idx
    ON sync_operation_archive(entity_id, canonical_revision, operation_id);

CREATE TABLE sync_tombstone (
    entity_id TEXT PRIMARY KEY NOT NULL CHECK (length(trim(entity_id)) > 0),
    entity_version INTEGER NOT NULL CHECK (entity_version >= 0),
    deleted_revision INTEGER NOT NULL CHECK (deleted_revision >= 0),
    deleted_at TEXT NOT NULL CHECK (length(trim(deleted_at)) > 0)
);

CREATE TABLE sync_conflict_vector (
    entity_id TEXT NOT NULL CHECK (length(trim(entity_id)) > 0),
    actor_id TEXT NOT NULL CHECK (length(trim(actor_id)) > 0),
    counter INTEGER NOT NULL CHECK (counter >= 0),
    PRIMARY KEY (entity_id, actor_id)
);

CREATE INDEX sync_conflict_vector_actor_idx
    ON sync_conflict_vector(actor_id, entity_id);
";

/// Immutable migration that creates the local Sync persistence schema.
pub const SYNC_MIGRATION: Migration = Migration {
    version: 3,
    name: "sync",
    sql: SYNC_SCHEMA_SQL,
};

/// The standalone Sync migration definition.
pub const SYNC_MIGRATIONS: &[Migration] = &[SYNC_MIGRATION];

#[cfg(test)]
mod tests {
    use rusqlite::{Connection, params};

    use super::{SYNC_MIGRATION, SYNC_MIGRATIONS, SYNC_SCHEMA_SQL};
    use crate::migration::apply_migrations;

    fn connection() -> Connection {
        let connection = Connection::open_in_memory().expect("in-memory SQLite");
        connection
            .execute_batch(SYNC_SCHEMA_SQL)
            .expect("Sync schema creates");
        connection
    }

    fn table_columns(connection: &Connection, table: &str) -> Vec<String> {
        let mut statement = connection
            .prepare(&format!("PRAGMA table_info({table})"))
            .expect("table info statement");
        statement
            .query_map([], |row| row.get(1))
            .expect("table info rows")
            .collect::<rusqlite::Result<_>>()
            .expect("table info collection")
    }

    #[test]
    fn migration_definition_is_version_three_and_append_only() {
        assert_eq!(SYNC_MIGRATION.version, 3);
        assert_eq!(SYNC_MIGRATION.name, "sync");
        assert_eq!(SYNC_MIGRATIONS, &[SYNC_MIGRATION]);
        assert!(!SYNC_SCHEMA_SQL.to_ascii_uppercase().contains("DROP TABLE"));
        assert!(!SYNC_SCHEMA_SQL.to_ascii_uppercase().contains("ALTER TABLE"));
    }

    #[test]
    fn schema_has_deterministic_tables_and_columns() {
        let connection = connection();
        for table in [
            "sync_replicated_cursor",
            "sync_outbox",
            "sync_operation_archive",
            "sync_tombstone",
            "sync_conflict_vector",
        ] {
            let exists: bool = connection
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1)",
                    [table],
                    |row| row.get(0),
                )
                .expect("table lookup");
            assert!(exists, "missing table {table}");
        }

        assert_eq!(
            table_columns(&connection, "sync_replicated_cursor"),
            vec!["id", "sync_epoch", "revision", "updated_at"]
        );
        assert_eq!(
            table_columns(&connection, "sync_outbox"),
            vec![
                "operation_id",
                "entity_id",
                "operation_bytes",
                "operation_digest",
                "base_entity_version",
                "state",
                "created_at",
                "acknowledged_at",
                "canonical_revision",
                "canonical_entity_version",
            ]
        );
        assert_eq!(
            table_columns(&connection, "sync_operation_archive"),
            vec![
                "operation_id",
                "entity_id",
                "operation_bytes",
                "operation_digest",
                "outcome",
                "canonical_revision",
                "archived_at",
            ]
        );
        assert_eq!(
            table_columns(&connection, "sync_tombstone"),
            vec![
                "entity_id",
                "entity_version",
                "deleted_revision",
                "deleted_at"
            ]
        );
        assert_eq!(
            table_columns(&connection, "sync_conflict_vector"),
            vec!["entity_id", "actor_id", "counter"]
        );
    }

    #[test]
    fn cursor_is_singleton_and_outbox_retains_exact_bytes() {
        let connection = connection();
        connection
            .execute(
                "INSERT INTO sync_replicated_cursor (id, sync_epoch, revision, updated_at) VALUES (1, 'epoch-a', 0, '2026-08-27T00:00:00Z')",
                [],
            )
            .expect("cursor insert");
        assert!(connection
            .execute(
                "INSERT INTO sync_replicated_cursor (id, sync_epoch, revision, updated_at) VALUES (2, 'epoch-b', 1, '2026-08-27T00:00:01Z')",
                [],
            )
            .is_err());

        let bytes = vec![0_u8, 255, 1, 2];
        let digest = vec![7_u8; 32];
        connection
            .execute(
                "INSERT INTO sync_outbox (operation_id, entity_id, operation_bytes, operation_digest, created_at) VALUES (?1, ?2, ?3, ?4, ?5)",
                params!["op-1", "entity-1", bytes, digest, "2026-08-27T00:00:00Z"],
            )
            .expect("outbox insert");
        let retained: (Vec<u8>, Vec<u8>, String) = connection
            .query_row(
                "SELECT operation_bytes, operation_digest, state FROM sync_outbox WHERE operation_id = 'op-1'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .expect("outbox row");
        assert_eq!(retained.0, vec![0, 255, 1, 2]);
        assert_eq!(retained.1, vec![7; 32]);
        assert_eq!(retained.2, "pending");
    }

    #[test]
    fn constraints_cover_digest_status_tombstone_and_vector_uniqueness() {
        let connection = connection();
        let invalid_rows = [
            "INSERT INTO sync_outbox (operation_id, entity_id, operation_bytes, operation_digest, created_at) VALUES ('op', 'entity', X'00', X'01', 't')",
            "INSERT INTO sync_outbox (operation_id, entity_id, operation_bytes, operation_digest, state, created_at) VALUES ('op', 'entity', X'00', zeroblob(32), 'unknown', 't')",
            "INSERT INTO sync_tombstone (entity_id, entity_version, deleted_revision, deleted_at) VALUES ('entity', -1, 1, 't')",
            "INSERT INTO sync_conflict_vector (entity_id, actor_id, counter) VALUES ('entity', 'actor', -1)",
        ];
        for sql in invalid_rows {
            assert!(
                connection.execute(sql, []).is_err(),
                "accepted invalid SQL: {sql}"
            );
        }

        connection
            .execute(
                "INSERT INTO sync_conflict_vector (entity_id, actor_id, counter) VALUES ('entity', 'actor', 1)",
                [],
            )
            .expect("vector insert");
        assert!(connection
            .execute(
                "INSERT INTO sync_conflict_vector (entity_id, actor_id, counter) VALUES ('entity', 'actor', 2)",
                [],
            )
            .is_err());
    }

    #[test]
    fn migration_applies_through_the_shared_ledger_and_is_idempotent() {
        let mut connection = Connection::open_in_memory().expect("in-memory SQLite");
        apply_migrations(&mut connection, SYNC_MIGRATIONS).expect("Sync migration applies");
        apply_migrations(&mut connection, SYNC_MIGRATIONS).expect("Sync migration is idempotent");
        let applied: (i64, String) = connection
            .query_row(
                "SELECT version, name FROM aurorix_schema_migrations",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .expect("migration ledger row");
        assert_eq!(applied, (3, "sync".to_owned()));
    }
}
