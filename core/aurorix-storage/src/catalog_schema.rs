//! Schema for the device-local music catalog and filesystem observations.
//!
//! The catalog is deliberately separate from replicated account state. An
//! asset row describes bytes observed on this device; changing or losing that
//! asset must not remove a recording or any user-owned playlist, favorite, or
//! play fact. Those user-owned tables are outside this migration.

use crate::migration::Migration;

/// SQL for the first local catalog schema migration.
///
/// The statement order is stable because the migration checksum includes the
/// complete SQL source. Foreign keys from an asset to a recording use
/// `SET NULL`, and no table containing a music identity is owned by an asset.
pub const LOCAL_CATALOG_SCHEMA_SQL: &str = r"
CREATE TABLE local_catalog_directory (
    id TEXT PRIMARY KEY NOT NULL,
    locator TEXT NOT NULL UNIQUE CHECK (length(trim(locator)) > 0),
    platform TEXT NOT NULL CHECK (length(trim(platform)) > 0),
    enabled INTEGER NOT NULL DEFAULT 1 CHECK (enabled IN (0, 1))
);

CREATE TABLE local_catalog_work (
    id TEXT PRIMARY KEY NOT NULL,
    title TEXT NOT NULL CHECK (length(trim(title)) > 0)
);

CREATE TABLE local_catalog_recording (
    id TEXT PRIMARY KEY NOT NULL,
    title TEXT NOT NULL CHECK (length(trim(title)) > 0),
    duration_ms INTEGER CHECK (duration_ms IS NULL OR duration_ms > 0)
);

CREATE TABLE local_catalog_recording_work (
    recording_id TEXT NOT NULL REFERENCES local_catalog_recording(id) ON DELETE CASCADE,
    work_id TEXT NOT NULL REFERENCES local_catalog_work(id) ON DELETE CASCADE,
    PRIMARY KEY (recording_id, work_id)
);

CREATE TABLE local_catalog_release (
    id TEXT PRIMARY KEY NOT NULL,
    title TEXT NOT NULL CHECK (length(trim(title)) > 0)
);

CREATE TABLE local_catalog_release_medium (
    id TEXT PRIMARY KEY NOT NULL,
    release_id TEXT NOT NULL REFERENCES local_catalog_release(id) ON DELETE CASCADE,
    medium_number INTEGER NOT NULL CHECK (medium_number > 0),
    UNIQUE (release_id, medium_number)
);

CREATE TABLE local_catalog_release_track (
    id TEXT PRIMARY KEY NOT NULL,
    medium_id TEXT NOT NULL REFERENCES local_catalog_release_medium(id) ON DELETE CASCADE,
    medium_number INTEGER NOT NULL CHECK (medium_number > 0),
    track_number INTEGER NOT NULL CHECK (track_number > 0),
    track_order INTEGER NOT NULL CHECK (track_order > 0),
    title TEXT NOT NULL CHECK (length(trim(title)) > 0),
    primary_recording_id TEXT REFERENCES local_catalog_recording(id) ON DELETE SET NULL,
    duration_ms INTEGER CHECK (duration_ms IS NULL OR duration_ms > 0),
    UNIQUE (medium_id, track_number),
    UNIQUE (medium_id, track_order)
);

CREATE TABLE local_catalog_scan (
    id TEXT PRIMARY KEY NOT NULL,
    started_at INTEGER NOT NULL,
    finished_at INTEGER,
    state TEXT NOT NULL DEFAULT 'running'
        CHECK (state IN ('running', 'completed', 'failed')),
    CHECK (finished_at IS NULL OR finished_at >= started_at)
);

CREATE TABLE local_catalog_asset (
    id TEXT PRIMARY KEY NOT NULL,
    directory_id TEXT REFERENCES local_catalog_directory(id) ON DELETE SET NULL,
    primary_recording_id TEXT REFERENCES local_catalog_recording(id) ON DELETE SET NULL,
    locator TEXT NOT NULL CHECK (length(trim(locator)) > 0),
    platform_file_id TEXT,
    size_bytes INTEGER CHECK (size_bytes IS NULL OR size_bytes >= 0),
    mtime_ns INTEGER CHECK (mtime_ns IS NULL OR mtime_ns >= 0),
    quick_hash TEXT,
    content_hash TEXT,
    fingerprint_algorithm TEXT,
    fingerprint_version TEXT,
    duration_ms INTEGER CHECK (duration_ms IS NULL OR duration_ms > 0),
    last_seen_scan_id TEXT REFERENCES local_catalog_scan(id) ON DELETE SET NULL,
    state TEXT NOT NULL DEFAULT 'discovered'
        CHECK (state IN (
            'discovered', 'probing', 'active', 'changed', 'missing',
            'permission_denied', 'unsupported', 'error',
            'relink_candidate', 'tombstoned'
        )),
    UNIQUE (directory_id, locator)
);

CREATE TABLE local_catalog_asset_segment (
    asset_id TEXT NOT NULL REFERENCES local_catalog_asset(id) ON DELETE CASCADE,
    segment_order INTEGER NOT NULL CHECK (segment_order > 0),
    title TEXT NOT NULL CHECK (length(trim(title)) > 0),
    start_ms INTEGER NOT NULL CHECK (start_ms >= 0),
    duration_ms INTEGER NOT NULL CHECK (duration_ms > 0),
    recording_id TEXT REFERENCES local_catalog_recording(id) ON DELETE SET NULL,
    PRIMARY KEY (asset_id, segment_order)
);

CREATE INDEX local_catalog_asset_recording_idx
    ON local_catalog_asset(primary_recording_id);
CREATE INDEX local_catalog_asset_state_idx
    ON local_catalog_asset(state);
CREATE INDEX local_catalog_asset_locator_idx
    ON local_catalog_asset(locator);
CREATE INDEX local_catalog_asset_segment_recording_idx
    ON local_catalog_asset_segment(recording_id);
";

/// Immutable migration that creates the device-local catalog schema.
pub const LOCAL_CATALOG_MIGRATION: Migration = Migration {
    version: 1,
    name: "local_catalog",
    sql: LOCAL_CATALOG_SCHEMA_SQL,
};

/// The ordered migrations currently required for local catalog storage.
pub const LOCAL_CATALOG_MIGRATIONS: &[Migration] = &[
    LOCAL_CATALOG_MIGRATION,
    crate::stats_schema::PLAY_STATS_MIGRATION,
];

#[cfg(test)]
mod tests {
    use rusqlite::Connection;

    use super::{LOCAL_CATALOG_MIGRATION, LOCAL_CATALOG_MIGRATIONS, LOCAL_CATALOG_SCHEMA_SQL};
    use crate::migration::apply_migrations;

    #[test]
    fn migration_definition_is_stable_and_contains_no_search_tables() {
        assert_eq!(LOCAL_CATALOG_MIGRATION.version, 1);
        assert_eq!(LOCAL_CATALOG_MIGRATION.name, "local_catalog");
        assert!(LOCAL_CATALOG_SCHEMA_SQL.contains("local_catalog_asset"));
        assert!(!LOCAL_CATALOG_SCHEMA_SQL.contains("VIRTUAL TABLE"));
        assert_eq!(LOCAL_CATALOG_MIGRATIONS.len(), 2);
        assert_eq!(LOCAL_CATALOG_MIGRATIONS[0], LOCAL_CATALOG_MIGRATION);
        assert_eq!(
            LOCAL_CATALOG_MIGRATIONS[1],
            crate::stats_schema::PLAY_STATS_MIGRATION
        );
    }

    #[test]
    fn migration_creates_catalog_tables_and_asset_loss_keeps_recording() {
        let mut connection = Connection::open_in_memory().expect("in-memory SQLite");
        connection
            .execute_batch("PRAGMA foreign_keys = ON")
            .expect("enable foreign keys");
        apply_migrations(&mut connection, LOCAL_CATALOG_MIGRATIONS)
            .expect("catalog migration applies");

        for table in [
            "local_catalog_directory",
            "local_catalog_work",
            "local_catalog_recording",
            "local_catalog_recording_work",
            "local_catalog_release",
            "local_catalog_release_medium",
            "local_catalog_release_track",
            "local_catalog_scan",
            "local_catalog_asset",
            "local_catalog_asset_segment",
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

        connection
            .execute(
                "INSERT INTO local_catalog_recording (id, title) VALUES ('recording-1', 'Track')",
                [],
            )
            .expect("recording insert");
        connection
            .execute(
                "INSERT INTO local_catalog_asset (id, primary_recording_id, locator, state) VALUES ('asset-1', 'recording-1', 'file:///track.flac', 'active')",
                [],
            )
            .expect("asset insert");
        connection
            .execute(
                "UPDATE local_catalog_asset SET state = 'missing' WHERE id = 'asset-1'",
                [],
            )
            .expect("asset state update");

        let recording_title: String = connection
            .query_row(
                "SELECT title FROM local_catalog_recording WHERE id = 'recording-1'",
                [],
                |row| row.get(0),
            )
            .expect("recording survives missing asset");
        assert_eq!(recording_title, "Track");
    }

    #[test]
    fn asset_state_and_metadata_constraints_are_deterministic() {
        let mut connection = Connection::open_in_memory().expect("in-memory SQLite");
        connection
            .execute_batch("PRAGMA foreign_keys = ON")
            .expect("enable foreign keys");
        apply_migrations(&mut connection, LOCAL_CATALOG_MIGRATIONS)
            .expect("catalog migration applies");

        let invalid_state = connection.execute(
            "INSERT INTO local_catalog_asset (id, locator, state) VALUES ('asset-1', 'file:///track.flac', 'gone')",
            [],
        );
        assert!(invalid_state.is_err());

        let invalid_duration = connection.execute(
            "INSERT INTO local_catalog_recording (id, title, duration_ms) VALUES ('recording-1', 'Track', 0)",
            [],
        );
        assert!(invalid_duration.is_err());
    }
}
