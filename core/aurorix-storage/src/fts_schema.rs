//! FTS5 schema for the device-local catalog search projection.
//!
//! This module owns only the `SQLite` search index shape. It is intentionally
//! separate from the catalog migration and does not add Provider, account,
//! Sync, credentials, or filesystem locator data to the searchable document.
//! Callers may execute [`LOCAL_CATALOG_FTS_SQL`] after the catalog tables exist,
//! then use [`REBUILD_LOCAL_CATALOG_FTS_SQL`] after a bulk catalog import.

/// The pinned FTS5 tokenizer policy for local catalog text.
///
/// `unicode61` gives deterministic Unicode-aware tokenization across the
/// supported desktop and mobile `SQLite` builds. CJK segmentation remains an
/// application concern and can be represented by inserting the chosen
/// normalized text into these columns; the schema does not silently select a
/// platform-specific tokenizer.
pub const LOCAL_CATALOG_FTS_TOKENIZER: &str = "unicode61 remove_diacritics 2";

/// The application-side policy for CJK input before it is inserted into FTS.
///
/// `SQLite`'s `unicode61` tokenizer has no portable CJK word-break contract.
/// Scanner/import code must therefore apply the same pinned overlapping
/// bigram segmentation on every platform before writing `title`, `artist`,
/// `album`, or `genre`. The segmenter itself belongs to a later query/import
/// module; this schema only records the policy at the storage boundary.
pub const LOCAL_CATALOG_FTS_CJK_POLICY: &str = "application-side overlapping CJK bigrams";

/// SQL creating the local catalog FTS5 managed-content projection.
///
/// The document row is keyed by the catalog entity's stable local id. Four
/// explicit text fields are retained for future ranking: title, artist,
/// album, and genre. The current catalog migration populates title-bearing
/// entities only; artist and genre values may be empty until their catalog
/// metadata tables are introduced. No locator or Provider identity is indexed.
pub const LOCAL_CATALOG_FTS_SQL: &str = r"
CREATE VIRTUAL TABLE IF NOT EXISTS local_catalog_fts USING fts5(
    entity_id UNINDEXED,
    entity_kind UNINDEXED,
    title,
    artist,
    album,
    genre,
    tokenize = 'unicode61 remove_diacritics 2'
);
";

/// SQL that discards the current FTS projection before deterministic reinsertion.
///
/// Reset is intentionally explicit and bounded to the FTS table. The
/// operation does not delete catalog rows or alter user-owned state. A caller
/// should execute this SQL in the same write transaction as a bulk projection
/// update so readers observe either the previous or the new index.
pub const REBUILD_LOCAL_CATALOG_FTS_SQL: &str = r"
DELETE FROM local_catalog_fts;
";

/// SQL removing all documents for one local catalog entity before reinsertion.
///
/// FTS5 does not provide a unique constraint over arbitrary stored columns.
/// Update code must therefore perform this delete and the corresponding
/// [`INSERT_LOCAL_CATALOG_FTS_SQL`] in one write transaction when replacing a
/// document. It never touches catalog or user-owned tables.
pub const DELETE_LOCAL_CATALOG_FTS_ENTITY_SQL: &str =
    "DELETE FROM local_catalog_fts WHERE entity_id = ?1;";

/// The FTS table's column order, used by query code to keep field weights
/// explicit and stable.
pub const LOCAL_CATALOG_FTS_COLUMNS: &[&str] = &["title", "artist", "album", "genre"];

/// Explicit `bm25` weights for the FTS columns, in table-column order.
///
/// The two identity columns are intentionally unindexed and receive zero
/// weight. Title is the strongest match, followed by artist, album, and genre.
pub const LOCAL_CATALOG_FTS_BM25_WEIGHTS: &[f64] = &[0.0, 0.0, 10.0, 6.0, 4.0, 2.0];

/// Inserts one normalized catalog document into the local FTS projection.
///
/// This helper is intentionally SQL-free at the API level: callers still own
/// the transaction and choose the local catalog entity identifiers. Empty
/// optional fields are represented as empty strings, which keeps query shape
/// deterministic without inventing placeholder text.
pub const INSERT_LOCAL_CATALOG_FTS_SQL: &str = r"
INSERT INTO local_catalog_fts(entity_id, entity_kind, title, artist, album, genre)
VALUES (?1, ?2, ?3, ?4, ?5, ?6);
";

#[cfg(test)]
mod tests {
    use rusqlite::{Connection, params};

    use super::{
        DELETE_LOCAL_CATALOG_FTS_ENTITY_SQL, INSERT_LOCAL_CATALOG_FTS_SQL,
        LOCAL_CATALOG_FTS_BM25_WEIGHTS, LOCAL_CATALOG_FTS_CJK_POLICY, LOCAL_CATALOG_FTS_COLUMNS,
        LOCAL_CATALOG_FTS_SQL, LOCAL_CATALOG_FTS_TOKENIZER, REBUILD_LOCAL_CATALOG_FTS_SQL,
    };

    fn connection() -> Connection {
        let connection = Connection::open_in_memory().expect("in-memory SQLite");
        connection
            .execute_batch(LOCAL_CATALOG_FTS_SQL)
            .expect("FTS5 schema creates");
        connection
    }

    #[test]
    fn schema_has_explicit_weighted_fields_and_pinned_tokenizer() {
        assert_eq!(
            LOCAL_CATALOG_FTS_COLUMNS,
            &["title", "artist", "album", "genre"]
        );
        assert_eq!(LOCAL_CATALOG_FTS_TOKENIZER, "unicode61 remove_diacritics 2");
        assert_eq!(
            LOCAL_CATALOG_FTS_CJK_POLICY,
            "application-side overlapping CJK bigrams"
        );
        assert_eq!(
            LOCAL_CATALOG_FTS_BM25_WEIGHTS,
            &[0.0, 0.0, 10.0, 6.0, 4.0, 2.0]
        );
        assert!(LOCAL_CATALOG_FTS_SQL.contains("entity_id UNINDEXED"));
        assert!(LOCAL_CATALOG_FTS_SQL.contains("entity_kind UNINDEXED"));
        assert!(LOCAL_CATALOG_FTS_SQL.contains("tokenize = 'unicode61 remove_diacritics 2'"));
        assert!(
            !LOCAL_CATALOG_FTS_SQL
                .to_ascii_lowercase()
                .contains("provider")
        );
        assert!(!LOCAL_CATALOG_FTS_SQL.to_ascii_lowercase().contains("sync"));
        assert!(
            !LOCAL_CATALOG_FTS_SQL
                .to_ascii_lowercase()
                .contains("locator")
        );
    }

    #[test]
    fn inserts_title_artist_album_and_genre_documents() {
        let connection = connection();
        connection
            .execute(
                INSERT_LOCAL_CATALOG_FTS_SQL,
                params![
                    "recording-1",
                    "recording",
                    "Dayvan Cowboy",
                    "Boards of Canada",
                    "The Campfire Headphase",
                    "electronic"
                ],
            )
            .expect("document insert");

        let rows: Vec<(String, String)> = connection
            .prepare(
                "SELECT entity_id, entity_kind FROM local_catalog_fts WHERE local_catalog_fts MATCH ?1 ORDER BY rowid",
            )
            .expect("search statement")
            .query_map(["title:Dayvan OR artist:Boards OR album:Campfire OR genre:electronic"], |row| {
                Ok((row.get(0)?, row.get(1)?))
            })
            .expect("search rows")
            .collect::<rusqlite::Result<_>>()
            .expect("collect search rows");

        assert_eq!(
            rows,
            vec![("recording-1".to_owned(), "recording".to_owned())]
        );
    }

    #[test]
    fn row_order_is_explicit_and_rebuild_clears_only_the_index() {
        let connection = connection();
        for (id, title) in [("b", "Beta"), ("a", "Alpha"), ("c", "Gamma")] {
            connection
                .execute(
                    INSERT_LOCAL_CATALOG_FTS_SQL,
                    params![id, "recording", title, "", "", ""],
                )
                .expect("document insert");
        }

        let ids = |connection: &Connection| {
            let mut statement = connection
                .prepare("SELECT entity_id FROM local_catalog_fts ORDER BY rowid")
                .expect("row-order statement");
            statement
                .query_map([], |row| row.get::<_, String>(0))
                .expect("row-order query")
                .collect::<rusqlite::Result<Vec<_>>>()
                .expect("row-order values")
        };

        assert_eq!(ids(&connection), vec!["b", "a", "c"]);
        connection
            .execute_batch("CREATE TABLE catalog_marker (value TEXT NOT NULL); INSERT INTO catalog_marker VALUES ('kept');")
            .expect("catalog marker creates");
        connection
            .execute_batch(REBUILD_LOCAL_CATALOG_FTS_SQL)
            .expect("rebuild succeeds");
        assert!(ids(&connection).is_empty());
        let marker: String = connection
            .query_row("SELECT value FROM catalog_marker", [], |row| row.get(0))
            .expect("catalog marker survives");
        assert_eq!(marker, "kept");
        connection
            .execute(
                INSERT_LOCAL_CATALOG_FTS_SQL,
                params!["a", "recording", "Alpha", "", "", ""],
            )
            .expect("reinsert document");
        connection
            .execute_batch(REBUILD_LOCAL_CATALOG_FTS_SQL)
            .expect("second rebuild succeeds");
        assert!(ids(&connection).is_empty());
    }

    #[test]
    fn duplicate_entity_ids_are_replaced_only_by_explicit_delete() {
        let connection = connection();
        connection
            .execute(
                INSERT_LOCAL_CATALOG_FTS_SQL,
                params!["recording-1", "recording", "Old title", "", "", ""],
            )
            .expect("initial insert");
        connection
            .execute(DELETE_LOCAL_CATALOG_FTS_ENTITY_SQL, ["recording-1"])
            .expect("explicit entity delete");
        connection
            .execute(
                INSERT_LOCAL_CATALOG_FTS_SQL,
                params!["recording-1", "recording", "New title", "", "", ""],
            )
            .expect("replacement insert");
        let count: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM local_catalog_fts WHERE entity_id = 'recording-1'",
                [],
                |row| row.get(0),
            )
            .expect("count rows");
        assert_eq!(count, 1);
        let title: String = connection
            .query_row(
                "SELECT title FROM local_catalog_fts WHERE entity_id = 'recording-1'",
                [],
                |row| row.get(0),
            )
            .expect("replacement row");
        assert_eq!(title, "New title");
    }
}
