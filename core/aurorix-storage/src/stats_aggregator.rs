//! Transactional aggregation of immutable playback facts.
//!
//! This module owns the local, rebuildable statistics projection. Callers
//! provide the fact identity, an opaque media projection key, and validated
//! timestamps. The applied-fact ledger and aggregate update happen in one
//! transaction so retries are idempotent.

use rusqlite::{Transaction, params};

use crate::stats_schema::{
    CLEAR_PLAY_STATS_PROJECTION_SQL, INSERT_PLAY_STATS_APPLIED_FACT_SQL, RESET_PLAY_STATS_SQL,
};

/// The result of applying one playback fact to the local projection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApplyPlayFactResult {
    /// The fact was newly recorded and contributed to the aggregate.
    Applied,
    /// The fact ID was already recorded; no aggregate state changed.
    AlreadyApplied,
}

/// Applies one immutable playback fact to the local statistics projection.
///
/// `fact_id` is the idempotency key. `media_key` is an opaque canonical key
/// chosen by the caller. `applied_at` is the ingestion timestamp, while
/// `played_at` is the playback timestamp used for `last_played_at`. Timestamp
/// ordering is delegated to `SQLite`'s text comparison, so callers must use one
/// lexically sortable representation (for example, normalized RFC 3339 UTC).
///
/// The marker insert and aggregate update are performed against the supplied
/// transaction. If the marker already exists, this function returns
/// [`ApplyPlayFactResult::AlreadyApplied`] and does not modify either table.
///
/// # Errors
///
/// Returns the underlying [`rusqlite::Error`] when an insert or aggregate
/// update is rejected by `SQLite` (including schema validation failures).
pub fn apply_play_fact(
    transaction: &Transaction<'_>,
    fact_id: &str,
    media_key: &str,
    applied_at: &str,
    played_ms: u64,
    played_at: &str,
) -> rusqlite::Result<ApplyPlayFactResult> {
    let played_ms = i64::try_from(played_ms).map_err(|_| {
        rusqlite::Error::ToSqlConversionFailure(Box::new(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "played duration exceeds SQLite integer range",
        )))
    })?;
    let inserted = transaction.execute(
        INSERT_PLAY_STATS_APPLIED_FACT_SQL,
        params![fact_id, applied_at],
    )?;
    if inserted == 0 {
        return Ok(ApplyPlayFactResult::AlreadyApplied);
    }

    transaction.execute(
        "INSERT INTO play_stats (media_key, play_count, played_ms, last_played_at)
         VALUES (?1, 1, ?2, ?3)
         ON CONFLICT(media_key) DO UPDATE SET
             play_count = play_stats.play_count + 1,
             played_ms = play_stats.played_ms + excluded.played_ms,
             last_played_at = CASE
                 WHEN play_stats.last_played_at IS NULL
                      OR excluded.last_played_at > play_stats.last_played_at
                 THEN excluded.last_played_at
                 ELSE play_stats.last_played_at
             END",
        params![media_key, played_ms, played_at],
    )?;
    Ok(ApplyPlayFactResult::Applied)
}

/// Clears only the derived aggregate rows.
///
/// Applied-fact markers remain so subsequent ingestion retries stay
/// idempotent. Use [`reset_play_stats`] before replaying the complete fact set.
///
/// # Errors
///
/// Returns the underlying [`rusqlite::Error`] when `SQLite` rejects the delete.
pub fn clear_play_stats(transaction: &Transaction<'_>) -> rusqlite::Result<usize> {
    transaction.execute(CLEAR_PLAY_STATS_PROJECTION_SQL, [])
}

/// Clears both the derived aggregate and its applied-fact ledger.
///
/// Callers should use this only when they will replay the complete immutable
/// fact set in the same bounded write workflow.
///
/// # Errors
///
/// Returns the underlying [`rusqlite::Error`] when `SQLite` rejects either
/// delete.
pub fn reset_play_stats(transaction: &Transaction<'_>) -> rusqlite::Result<()> {
    transaction.execute_batch(RESET_PLAY_STATS_SQL)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{ApplyPlayFactResult, apply_play_fact, clear_play_stats, reset_play_stats};
    use crate::{catalog_schema::LOCAL_CATALOG_MIGRATIONS, migration::apply_migrations};
    use rusqlite::Connection;

    fn database() -> Connection {
        let mut connection = Connection::open_in_memory().expect("in-memory SQLite");
        connection
            .execute_batch("PRAGMA foreign_keys = ON")
            .expect("enable foreign keys");
        apply_migrations(&mut connection, LOCAL_CATALOG_MIGRATIONS)
            .expect("catalog and statistics migrations apply");
        connection
    }

    fn apply(
        connection: &mut Connection,
        fact_id: &str,
        media_key: &str,
        played_ms: u64,
        played_at: &str,
    ) -> ApplyPlayFactResult {
        let transaction = connection.transaction().expect("transaction");
        let result = apply_play_fact(
            &transaction,
            fact_id,
            media_key,
            "2026-08-27T12:00:00Z",
            played_ms,
            played_at,
        )
        .expect("fact applies");
        transaction.commit().expect("commit");
        result
    }

    #[test]
    fn first_fact_creates_one_aggregate_row() {
        let mut connection = database();
        assert_eq!(
            apply(
                &mut connection,
                "fact-1",
                "media:one",
                120_000,
                "2026-08-27T12:10:00Z"
            ),
            ApplyPlayFactResult::Applied
        );
        let row: (i64, i64, String) = connection
            .query_row(
                "SELECT play_count, played_ms, last_played_at FROM play_stats WHERE media_key = 'media:one'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .expect("aggregate row");
        assert_eq!(row, (1, 120_000, "2026-08-27T12:10:00Z".to_owned()));
    }

    #[test]
    fn duplicate_fact_does_not_change_aggregate() {
        let mut connection = database();
        apply(
            &mut connection,
            "fact-1",
            "media:one",
            120_000,
            "2026-08-27T12:10:00Z",
        );
        assert_eq!(
            apply(
                &mut connection,
                "fact-1",
                "media:one",
                999_000,
                "2026-08-27T13:10:00Z"
            ),
            ApplyPlayFactResult::AlreadyApplied
        );
        let row: (i64, i64, String) = connection
            .query_row(
                "SELECT play_count, played_ms, last_played_at FROM play_stats WHERE media_key = 'media:one'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .expect("aggregate row");
        assert_eq!(row, (1, 120_000, "2026-08-27T12:10:00Z".to_owned()));
    }

    #[test]
    fn distinct_facts_accumulate_and_keep_newest_playback_time() {
        let mut connection = database();
        apply(
            &mut connection,
            "fact-1",
            "media:one",
            100,
            "2026-08-27T12:10:00Z",
        );
        apply(
            &mut connection,
            "fact-2",
            "media:one",
            200,
            "2026-08-27T12:09:00Z",
        );
        apply(
            &mut connection,
            "fact-3",
            "media:one",
            300,
            "2026-08-27T12:11:00Z",
        );
        let row: (i64, i64, String) = connection
            .query_row(
                "SELECT play_count, played_ms, last_played_at FROM play_stats WHERE media_key = 'media:one'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .expect("aggregate row");
        assert_eq!(row, (3, 600, "2026-08-27T12:11:00Z".to_owned()));
    }

    #[test]
    fn clear_and_reset_have_distinct_ledger_semantics() {
        let mut connection = database();
        apply(
            &mut connection,
            "fact-1",
            "media:one",
            100,
            "2026-08-27T12:10:00Z",
        );
        let transaction = connection.transaction().expect("transaction");
        assert_eq!(clear_play_stats(&transaction).expect("clear projection"), 1);
        transaction.commit().expect("commit");
        let applied_count: i64 = connection
            .query_row("SELECT COUNT(*) FROM play_stats_applied_facts", [], |row| {
                row.get(0)
            })
            .expect("marker count");
        assert_eq!(applied_count, 1);

        let transaction = connection.transaction().expect("transaction");
        reset_play_stats(&transaction).expect("reset projection");
        transaction.commit().expect("commit");
        let marker_count: i64 = connection
            .query_row("SELECT COUNT(*) FROM play_stats_applied_facts", [], |row| {
                row.get(0)
            })
            .expect("marker count");
        assert_eq!(marker_count, 0);
    }
}
