//! Bounded local FTS5 search over the storage-owned projection.

use aurorix_app::{
    error::{AppError, AppErrorCode, RequestCorrelationId},
    pagination::KeysetCursor,
    search_projection::{
        LocalSearchAvailability, LocalSearchHit, LocalSearchPage, LocalSearchResultKind,
    },
    search_query::LocalSearchRequest,
};
use rusqlite::Connection;

/// Executes local FTS queries against a checked `SQLite` connection.
pub struct LocalFtsSearch<'a> {
    connection: &'a Connection,
}

impl<'a> LocalFtsSearch<'a> {
    /// Creates a search adapter over an existing connection.
    #[must_use]
    pub const fn new(connection: &'a Connection) -> Self {
        Self { connection }
    }
}

impl LocalFtsSearch<'_> {
    /// Executes one bounded local search page.
    ///
    /// # Errors
    ///
    /// Returns an [`AppError`] with the supplied request identifier when the
    /// FTS query or row projection fails.
    pub fn search(
        &self,
        request: &LocalSearchRequest,
        request_id: &RequestCorrelationId,
    ) -> Result<LocalSearchPage, AppError> {
        let query = request.search_text().as_str();
        let limit = i64::from(request.page().page_size().get());
        let cursor = request
            .page()
            .cursor()
            .map(|value| decode_cursor(value.as_str()))
            .transpose()
            .map_err(|()| AppError::new(AppErrorCode::InvalidRequest, request_id.clone()))?;
        let cursor_rank = cursor.as_ref().map(|value| value.0);
        let cursor_id = cursor.as_ref().map(|value| value.1.as_str());
        let mut statement = self
            .connection
            .prepare(
                "WITH ranked AS (
                    SELECT entity_kind, entity_id, title, artist, album, genre,
                        bm25(local_catalog_fts, 0.0, 0.0, 10.0, 6.0, 4.0, 2.0) AS rank
                    FROM local_catalog_fts
                    WHERE local_catalog_fts MATCH ?1
                )
                SELECT entity_kind, entity_id, title, artist, album, genre, rank
                FROM ranked
                WHERE (?2 IS NULL OR rank > ?2 OR (rank = ?2 AND entity_id > ?3))
                ORDER BY rank ASC, entity_id ASC
                LIMIT ?4",
            )
            .map_err(|_| AppError::new(AppErrorCode::Unavailable, request_id.clone()))?;
        let rows = statement
            .query_map((query, cursor_rank, cursor_id, limit), |row| {
                let kind_value = row.get::<_, String>(0)?;
                let kind = parse_kind(&kind_value).ok_or_else(|| {
                    rusqlite::Error::InvalidColumnType(
                        0,
                        "entity_kind".to_owned(),
                        rusqlite::types::Type::Text,
                    )
                })?;
                let id = row.get::<_, String>(1)?.parse().map_err(|_| {
                    rusqlite::Error::InvalidColumnType(
                        1,
                        "entity_id".to_owned(),
                        rusqlite::types::Type::Text,
                    )
                })?;
                let hit = LocalSearchHit::new(
                    kind,
                    id,
                    row.get::<_, String>(2)?,
                    nonempty_artist(row.get::<_, String>(3)?),
                    optional_field(row.get::<_, String>(4)?),
                    None,
                    LocalSearchAvailability::Available,
                )
                .map_err(|_| rusqlite::Error::InvalidQuery)?;
                let rank = row.get::<_, f64>(6)?;
                Ok((hit, rank))
            })
            .map_err(|_| AppError::new(AppErrorCode::Unavailable, request_id.clone()))?;
        let ranked_hits = rows
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|_| AppError::new(AppErrorCode::Unavailable, request_id.clone()))?;
        let next_cursor = if ranked_hits.len() == request.page().page_size().get() as usize {
            ranked_hits
                .last()
                .map(|(hit, rank)| encode_cursor(*rank, &hit.local_id().to_string()))
        } else {
            None
        };
        let hits = ranked_hits.into_iter().map(|(hit, _)| hit).collect();
        Ok(LocalSearchPage::new(hits, next_cursor))
    }
}

fn optional_field(value: String) -> Option<String> {
    (!value.is_empty()).then_some(value)
}

fn nonempty_artist(value: String) -> Vec<String> {
    optional_field(value).into_iter().collect()
}

fn parse_kind(value: &str) -> Option<LocalSearchResultKind> {
    match value {
        "recording" => Some(LocalSearchResultKind::Recording),
        "release" => Some(LocalSearchResultKind::Release),
        "work" => Some(LocalSearchResultKind::Work),
        _ => None,
    }
}

fn encode_cursor(rank: f64, entity_id: &str) -> KeysetCursor {
    KeysetCursor::new(format!("bm25:{:016x}:{entity_id}", rank.to_bits()))
        .expect("encoded BM25 cursor is non-empty")
}

fn decode_cursor(value: &str) -> Result<(f64, String), ()> {
    let mut parts = value.splitn(3, ':');
    if parts.next() != Some("bm25") {
        return Err(());
    }
    let bits = u64::from_str_radix(parts.next().ok_or(())?, 16).map_err(|_| ())?;
    let entity_id = parts.next().ok_or(())?;
    if entity_id.is_empty() {
        return Err(());
    }
    let rank = f64::from_bits(bits);
    rank.is_finite()
        .then_some((rank, entity_id.to_owned()))
        .ok_or(())
}

#[cfg(test)]
mod tests {
    use super::LocalFtsSearch;
    use crate::fts_schema::{INSERT_LOCAL_CATALOG_FTS_SQL, LOCAL_CATALOG_FTS_SQL};
    use aurorix_app::{
        error::RequestCorrelationId,
        pagination::SearchPageRequest,
        search_query::{LocalSearchRequest, SearchText},
    };
    use rusqlite::{Connection, params};

    #[test]
    fn search_returns_stable_bounded_results() {
        let connection = Connection::open_in_memory().unwrap();
        connection.execute_batch(LOCAL_CATALOG_FTS_SQL).unwrap();
        connection
            .execute(
                INSERT_LOCAL_CATALOG_FTS_SQL,
                params![
                    "550e8400-e29b-41d4-a716-446655440000",
                    "recording",
                    "Dayvan Cowboy",
                    "Boards of Canada",
                    "",
                    ""
                ],
            )
            .unwrap();
        let request = LocalSearchRequest::new(
            SearchText::new("boards").unwrap(),
            SearchPageRequest::default(),
        );
        let id = RequestCorrelationId::new("request").unwrap();
        let page = LocalFtsSearch::new(&connection)
            .search(&request, &id)
            .unwrap();
        assert_eq!(page.hits().len(), 1);
        assert_eq!(page.hits()[0].title(), "Dayvan Cowboy");
    }
}
