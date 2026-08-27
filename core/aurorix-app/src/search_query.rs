//! Local-search text and pagination request values.
//!
//! This module validates user-entered search text and pairs it with bounded
//! keyset pagination. It does not define normalization, ordering, query
//! execution, result rows, Provider search, or storage behavior.

use std::{error::Error, fmt};

use crate::pagination::SearchPageRequest;

/// Validation failure for local-search text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SearchTextError {
    /// Search text had no non-whitespace characters.
    EmptySearchText,
}

impl fmt::Display for SearchTextError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptySearchText => formatter.write_str("search text must not be empty"),
        }
    }
}

impl Error for SearchTextError {}

/// Non-empty search text whose original value is preserved.
///
/// The value is deliberately not trimmed or normalized. Search execution owns
/// any later interpretation needed by its concrete implementation.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SearchText(String);

impl SearchText {
    /// Creates non-empty local-search text while preserving its original value.
    ///
    /// # Errors
    ///
    /// Returns [`SearchTextError::EmptySearchText`] when `value` has no
    /// non-whitespace characters.
    pub fn new(value: impl Into<String>) -> Result<Self, SearchTextError> {
        let value = value.into();
        if value.trim().is_empty() {
            return Err(SearchTextError::EmptySearchText);
        }

        Ok(Self(value))
    }

    /// Returns the original, unnormalized search text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for SearchText {
    type Error = SearchTextError;

    /// # Errors
    ///
    /// Returns [`SearchTextError::EmptySearchText`] when `value` has no
    /// non-whitespace characters.
    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl TryFrom<&str> for SearchText {
    type Error = SearchTextError;

    /// # Errors
    ///
    /// Returns [`SearchTextError::EmptySearchText`] when `value` has no
    /// non-whitespace characters.
    fn try_from(value: &str) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

/// A local-search request with validated text and bounded pagination.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalSearchRequest {
    search_text: SearchText,
    page: SearchPageRequest,
}

impl LocalSearchRequest {
    /// Creates a request from validated local-search text and pagination.
    #[must_use]
    pub fn new(search_text: SearchText, page: SearchPageRequest) -> Self {
        Self { search_text, page }
    }

    /// Returns the original, validated local-search text.
    #[must_use]
    pub fn search_text(&self) -> &SearchText {
        &self.search_text
    }

    /// Returns the bounded keyset pagination request.
    #[must_use]
    pub fn page(&self) -> &SearchPageRequest {
        &self.page
    }
}

#[cfg(test)]
mod tests {
    use crate::pagination::{KeysetCursor, PageSize};

    use super::{LocalSearchRequest, SearchText, SearchTextError};

    #[test]
    fn search_text_rejects_empty_or_whitespace_only_values() {
        assert_eq!(SearchText::new(""), Err(SearchTextError::EmptySearchText));
        assert_eq!(
            SearchText::new(" \t\n "),
            Err(SearchTextError::EmptySearchText)
        );
    }

    #[test]
    fn search_text_preserves_non_empty_original_values() {
        let search_text = SearchText::new("  Aphex Twin  ").expect("valid search text");

        assert_eq!(search_text.as_str(), "  Aphex Twin  ");
    }

    #[test]
    fn search_text_try_from_uses_the_same_validation() {
        assert_eq!(
            SearchText::try_from("\n"),
            Err(SearchTextError::EmptySearchText)
        );
    }

    #[test]
    fn local_search_request_keeps_validated_text_and_pagination() {
        let search_text = SearchText::new("Boards of Canada").expect("valid search text");
        let page_size = PageSize::new(75).expect("valid page size");
        let cursor = KeysetCursor::new("local-page-2").expect("valid cursor");
        let page = crate::pagination::SearchPageRequest::new(page_size, Some(cursor));
        let request = LocalSearchRequest::new(search_text, page);

        assert_eq!(request.search_text().as_str(), "Boards of Canada");
        assert_eq!(request.page().page_size(), page_size);
        assert_eq!(
            request.page().cursor().map(KeysetCursor::as_str),
            Some("local-page-2")
        );
    }
}
