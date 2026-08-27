//! Local catalog search-result projection values.
//!
//! These values describe already ordered local search results. They do not
//! define search execution, ranking, de-duplication, Provider results, or
//! storage behavior.

use std::{error::Error, fmt};

use aurorix_model::{ids::LocalCatalogEntityId, music::DurationMs};

use crate::pagination::KeysetCursor;

/// The local catalog entity represented by a [`LocalSearchHit`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LocalSearchResultKind {
    /// A musical work.
    Work,
    /// A recording.
    Recording,
    /// A release.
    Release,
    /// A position on a release medium.
    ReleaseTrack,
}

/// Whether a local search hit is currently available for local use.
///
/// This is a query projection, not a local-asset state machine. The storage
/// implementation decides how its detailed scan states map to this value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LocalSearchAvailability {
    /// The local catalog currently has an available representation.
    Available,
    /// The catalog item is known but currently has no available local representation.
    Unavailable,
}

/// Validation failure for a local search-result projection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LocalSearchProjectionError {
    /// A required display field had no non-whitespace characters.
    EmptyText {
        /// The rejected display field.
        field: &'static str,
    },
}

impl fmt::Display for LocalSearchProjectionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyText { field } => write!(formatter, "{field} must not be empty"),
        }
    }
}

impl Error for LocalSearchProjectionError {}

/// One local catalog result returned from an already ordered search.
///
/// Artist display names remain ordered and may be empty when local metadata
/// does not identify an artist. The hit does not carry a storage locator,
/// Provider identity, or an execution score.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalSearchHit {
    kind: LocalSearchResultKind,
    local_id: LocalCatalogEntityId,
    title: String,
    artists: Vec<String>,
    release_title: Option<String>,
    duration_ms: Option<DurationMs>,
    availability: LocalSearchAvailability,
}

impl LocalSearchHit {
    /// Creates a validated local search-result projection.
    ///
    /// The original display text is preserved. Artist names may be absent, but
    /// each supplied artist and an optional release title must be non-blank.
    ///
    /// # Errors
    ///
    /// Returns [`LocalSearchProjectionError::EmptyText`] when `title`, an
    /// artist name, or `release_title` has no non-whitespace characters.
    pub fn new(
        kind: LocalSearchResultKind,
        local_id: LocalCatalogEntityId,
        title: impl Into<String>,
        artists: Vec<String>,
        release_title: Option<String>,
        duration_ms: Option<DurationMs>,
        availability: LocalSearchAvailability,
    ) -> Result<Self, LocalSearchProjectionError> {
        let title = required_text(title.into(), "search result title")?;
        validate_artists(&artists)?;
        let release_title = release_title
            .map(|value| required_text(value, "search result release title"))
            .transpose()?;

        Ok(Self {
            kind,
            local_id,
            title,
            artists,
            release_title,
            duration_ms,
            availability,
        })
    }

    /// Returns the local catalog entity kind.
    #[must_use]
    pub const fn kind(&self) -> LocalSearchResultKind {
        self.kind
    }

    /// Returns the local catalog identifier for this result.
    #[must_use]
    pub const fn local_id(&self) -> LocalCatalogEntityId {
        self.local_id
    }

    /// Returns the local display title.
    #[must_use]
    pub fn title(&self) -> &str {
        &self.title
    }

    /// Returns ordered local artist display names.
    #[must_use]
    pub fn artists(&self) -> &[String] {
        &self.artists
    }

    /// Returns the local release title, when known.
    #[must_use]
    pub fn release_title(&self) -> Option<&str> {
        self.release_title.as_deref()
    }

    /// Returns the known duration, when catalog metadata provides one.
    #[must_use]
    pub const fn duration_ms(&self) -> Option<DurationMs> {
        self.duration_ms
    }

    /// Returns the projected local availability.
    #[must_use]
    pub const fn availability(&self) -> LocalSearchAvailability {
        self.availability
    }
}

/// One page of already ordered local search hits.
///
/// A continuation cursor is opaque and is meaningful only to the query
/// implementation that produced it. This value does not derive ordering or
/// infer whether another page exists.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalSearchPage {
    hits: Vec<LocalSearchHit>,
    next_cursor: Option<KeysetCursor>,
}

impl LocalSearchPage {
    /// Creates a local search-result page from already projected hits.
    #[must_use]
    pub fn new(hits: Vec<LocalSearchHit>, next_cursor: Option<KeysetCursor>) -> Self {
        Self { hits, next_cursor }
    }

    /// Returns the hits in the query implementation's established order.
    #[must_use]
    pub fn hits(&self) -> &[LocalSearchHit] {
        &self.hits
    }

    /// Returns the opaque continuation cursor, when the query produced one.
    #[must_use]
    pub fn next_cursor(&self) -> Option<&KeysetCursor> {
        self.next_cursor.as_ref()
    }
}

fn required_text(value: String, field: &'static str) -> Result<String, LocalSearchProjectionError> {
    if value.trim().is_empty() {
        return Err(LocalSearchProjectionError::EmptyText { field });
    }

    Ok(value)
}

fn validate_artists(artists: &[String]) -> Result<(), LocalSearchProjectionError> {
    for artist in artists {
        if artist.trim().is_empty() {
            return Err(LocalSearchProjectionError::EmptyText {
                field: "search result artist",
            });
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        LocalSearchAvailability, LocalSearchHit, LocalSearchPage, LocalSearchProjectionError,
        LocalSearchResultKind,
    };
    use crate::pagination::KeysetCursor;
    use aurorix_model::{ids::LocalCatalogEntityId, music::DurationMs};

    fn id() -> LocalCatalogEntityId {
        LocalCatalogEntityId::new_v7()
    }

    fn hit() -> LocalSearchHit {
        LocalSearchHit::new(
            LocalSearchResultKind::Recording,
            id(),
            "  Dayvan Cowboy  ",
            vec!["Boards of Canada".to_owned(), "  Guest Artist  ".to_owned()],
            Some("The Campfire Headphase".to_owned()),
            Some(DurationMs::new(300_000).expect("positive duration")),
            LocalSearchAvailability::Available,
        )
        .expect("valid local search hit")
    }

    #[test]
    fn search_hit_preserves_valid_local_projection_values() {
        let hit = hit();

        assert_eq!(hit.kind(), LocalSearchResultKind::Recording);
        assert_eq!(hit.title(), "  Dayvan Cowboy  ");
        assert_eq!(hit.artists(), ["Boards of Canada", "  Guest Artist  "]);
        assert_eq!(hit.release_title(), Some("The Campfire Headphase"));
        assert_eq!(hit.duration_ms().map(DurationMs::get), Some(300_000));
        assert_eq!(hit.availability(), LocalSearchAvailability::Available);
    }

    #[test]
    fn search_hit_allows_absent_optional_metadata() {
        let hit = LocalSearchHit::new(
            LocalSearchResultKind::Work,
            id(),
            "Unknown work",
            Vec::new(),
            None,
            None,
            LocalSearchAvailability::Unavailable,
        )
        .expect("valid local search hit with unknown metadata");

        assert!(hit.artists().is_empty());
        assert_eq!(hit.release_title(), None);
        assert_eq!(hit.duration_ms(), None);
        assert_eq!(hit.availability(), LocalSearchAvailability::Unavailable);
    }

    #[test]
    fn search_hit_rejects_blank_display_fields_without_normalizing_them() {
        assert_eq!(
            LocalSearchHit::new(
                LocalSearchResultKind::Recording,
                id(),
                " \t ",
                Vec::new(),
                None,
                None,
                LocalSearchAvailability::Available,
            ),
            Err(LocalSearchProjectionError::EmptyText {
                field: "search result title"
            })
        );
        assert_eq!(
            LocalSearchHit::new(
                LocalSearchResultKind::Recording,
                id(),
                "Title",
                vec!["\n".to_owned()],
                None,
                None,
                LocalSearchAvailability::Available,
            ),
            Err(LocalSearchProjectionError::EmptyText {
                field: "search result artist"
            })
        );
        assert_eq!(
            LocalSearchHit::new(
                LocalSearchResultKind::Recording,
                id(),
                "Title",
                Vec::new(),
                Some(" ".to_owned()),
                None,
                LocalSearchAvailability::Available,
            ),
            Err(LocalSearchProjectionError::EmptyText {
                field: "search result release title"
            })
        );
    }

    #[test]
    fn search_page_keeps_hit_order_and_opaque_continuation() {
        let cursor = KeysetCursor::new("local-cursor-2").expect("valid cursor");
        let page = LocalSearchPage::new(vec![hit()], Some(cursor));

        assert_eq!(page.hits().len(), 1);
        assert_eq!(page.hits()[0].title(), "  Dayvan Cowboy  ");
        assert_eq!(
            page.next_cursor().map(KeysetCursor::as_str),
            Some("local-cursor-2")
        );
    }
}
