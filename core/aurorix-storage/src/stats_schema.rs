//! `SQLite` schema for the device-local playback statistics projection.
//!
//! Playback facts are immutable domain values and remain the source of truth.
//! This module owns only the rebuildable aggregate and the per-fact idempotency
//! ledger used by a later aggregation transaction. It deliberately does not
//! define facts, Sync cursors, transport state, or account/provider data.

use crate::migration::Migration;

/// SQL creating the local playback statistics projection and its idempotency
/// ledger.
///
/// `media_key` is an opaque, canonical projection key supplied by the caller;
/// this schema does not impose a local catalog or Provider identity on it.
/// The `fact_id` primary key makes applying one immutable fact at most once.
pub const PLAY_STATS_SCHEMA_SQL: &str = r"
CREATE TABLE play_stats_applied_facts (
    fact_id TEXT PRIMARY KEY NOT NULL CHECK (length(trim(fact_id)) > 0),
    applied_at TEXT NOT NULL CHECK (length(trim(applied_at)) > 0)
);

CREATE TABLE play_stats (
    media_key TEXT PRIMARY KEY NOT NULL CHECK (length(trim(media_key)) > 0),
    play_count INTEGER NOT NULL DEFAULT 0 CHECK (play_count >= 0),
    played_ms INTEGER NOT NULL DEFAULT 0 CHECK (played_ms >= 0),
    last_played_at TEXT CHECK (last_played_at IS NULL OR length(trim(last_played_at)) > 0)
);

CREATE INDEX play_stats_last_played_idx
    ON play_stats(last_played_at DESC, media_key ASC);
CREATE INDEX play_stats_play_count_idx
    ON play_stats(play_count DESC, media_key ASC);
";

/// Immutable migration that creates the local playback statistics tables.
pub const PLAY_STATS_MIGRATION: Migration = Migration {
    version: 2,
    name: "play_stats",
    sql: PLAY_STATS_SCHEMA_SQL,
};

/// The standalone migration definition for playback statistics storage.
pub const PLAY_STATS_MIGRATIONS: &[Migration] = &[PLAY_STATS_MIGRATION];

/// Inserts an applied-fact marker only when the fact has not been seen before.
///
/// The caller should execute this statement and update `play_stats` only when
/// the returned row count is one, in the same transaction.
pub const INSERT_PLAY_STATS_APPLIED_FACT_SQL: &str = r"
INSERT INTO play_stats_applied_facts (fact_id, applied_at)
VALUES (?1, ?2)
ON CONFLICT(fact_id) DO NOTHING;
";

/// SQL for rebuilding the aggregate projection from immutable playback facts.
///
/// The applied-facts ledger is intentionally retained: it describes ingestion
/// idempotency and is not part of the derived aggregate. A caller rebuilding
/// from the complete immutable fact set should clear the ledger in the same
/// transaction only if it is also replaying those facts.
pub const CLEAR_PLAY_STATS_PROJECTION_SQL: &str = "DELETE FROM play_stats;";

/// SQL that resets both derived statistics state and its ingestion ledger.
///
/// Execute this only when the caller is going to replay the complete immutable
/// fact set in the same bounded write transaction. This prevents old markers
/// from suppressing the replay after a damaged projection is discarded.
pub const RESET_PLAY_STATS_SQL: &str = r"
DELETE FROM play_stats;
DELETE FROM play_stats_applied_facts;
";

#[cfg(test)]
mod tests {
    use rusqlite::{Connection, params};

    use super::{
        CLEAR_PLAY_STATS_PROJECTION_SQL, INSERT_PLAY_STATS_APPLIED_FACT_SQL, PLAY_STATS_MIGRATION,
        PLAY_STATS_MIGRATIONS, PLAY_STATS_SCHEMA_SQL, RESET_PLAY_STATS_SQL,
    };

    fn connection() -> Connection {
        let connection = Connection::open_in_memory().expect("in-memory SQLite");
        connection
            .execute_batch(PLAY_STATS_SCHEMA_SQL)
            .expect("statistics schema creates");
        connection
    }

    #[test]
    fn schema_has_fact_deduplication_and_rebuildable_projection() {
        let connection = connection();
        assert_eq!(PLAY_STATS_MIGRATION.version, 2);
        assert_eq!(PLAY_STATS_MIGRATION.name, "play_stats");
        assert_eq!(PLAY_STATS_MIGRATIONS, &[PLAY_STATS_MIGRATION]);
        for table in ["play_stats_applied_facts", "play_stats"] {
            let exists: bool = connection
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1)",
                    [table],
                    |row| row.get(0),
                )
                .expect("table lookup");
            assert!(exists, "missing table {table}");
        }

        let columns: Vec<String> = connection
            .prepare("PRAGMA table_info(play_stats)")
            .expect("table info statement")
            .query_map([], |row| row.get(1))
            .expect("table info rows")
            .collect::<rusqlite::Result<_>>()
            .expect("table info collection");
        assert_eq!(
            columns,
            vec![
                "media_key".to_owned(),
                "play_count".to_owned(),
                "played_ms".to_owned(),
                "last_played_at".to_owned()
            ]
        );
    }

    #[test]
    fn applied_fact_insert_is_idempotent_and_projection_can_be_cleared() {
        let connection = connection();
        assert_eq!(
            connection
                .execute(
                    INSERT_PLAY_STATS_APPLIED_FACT_SQL,
                    params!["fact-1", "2026-08-27T12:00:00Z"],
                )
                .expect("first fact marker"),
            1
        );
        assert_eq!(
            connection
                .execute(
                    INSERT_PLAY_STATS_APPLIED_FACT_SQL,
                    params!["fact-1", "2026-08-27T12:01:00Z"],
                )
                .expect("duplicate fact marker"),
            0
        );
        connection
            .execute(
                "INSERT INTO play_stats (media_key, play_count, played_ms, last_played_at) VALUES (?1, 1, 1000, ?2)",
                params!["media:one", "2026-08-27T12:00:00Z"],
            )
            .expect("aggregate row");
        connection
            .execute_batch(CLEAR_PLAY_STATS_PROJECTION_SQL)
            .expect("clear aggregate projection");

        let aggregate_count: i64 = connection
            .query_row("SELECT COUNT(*) FROM play_stats", [], |row| row.get(0))
            .expect("aggregate count");
        let applied_count: i64 = connection
            .query_row("SELECT COUNT(*) FROM play_stats_applied_facts", [], |row| {
                row.get(0)
            })
            .expect("applied fact count");
        assert_eq!(aggregate_count, 0);
        assert_eq!(applied_count, 1);

        connection
            .execute_batch(RESET_PLAY_STATS_SQL)
            .expect("reset aggregate and applied-fact ledger");
        let reset_applied_count: i64 = connection
            .query_row("SELECT COUNT(*) FROM play_stats_applied_facts", [], |row| {
                row.get(0)
            })
            .expect("reset applied fact count");
        assert_eq!(reset_applied_count, 0);
    }

    #[test]
    fn constraints_reject_invalid_statistics_values() {
        let connection = connection();
        for sql in [
            "INSERT INTO play_stats (media_key, play_count, played_ms) VALUES ('', 0, 0)",
            "INSERT INTO play_stats (media_key, play_count, played_ms) VALUES ('media:one', -1, 0)",
            "INSERT INTO play_stats (media_key, play_count, played_ms) VALUES ('media:two', 0, -1)",
            "INSERT INTO play_stats_applied_facts (fact_id, applied_at) VALUES ('', '2026-08-27T12:00:00Z')",
        ] {
            assert!(
                connection.execute(sql, []).is_err(),
                "accepted invalid SQL: {sql}"
            );
        }
    }
}
