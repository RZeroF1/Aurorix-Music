//! Bounded keyset-pagination values for local search requests.
//!
//! This module validates page sizing and carries opaque cursor values. It does
//! not define search ordering, SQL queries, result rows, or Provider paging.

use std::{error::Error, fmt};

/// The page size used by [`SearchPageRequest::default`].
pub const DEFAULT_PAGE_SIZE: u32 = 50;

/// The largest page size accepted by a local search request.
pub const MAX_PAGE_SIZE: u32 = 200;

/// Validation failure for a search-pagination value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PaginationError {
    /// A page size was zero.
    ZeroPageSize,
    /// A page size exceeded [`MAX_PAGE_SIZE`].
    PageSizeExceedsMaximum {
        /// The rejected page size.
        requested: u32,
        /// The accepted maximum page size.
        maximum: u32,
    },
    /// A cursor had no non-whitespace characters.
    EmptyCursor,
}

impl fmt::Display for PaginationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ZeroPageSize => formatter.write_str("page size must be greater than zero"),
            Self::PageSizeExceedsMaximum { requested, maximum } => {
                write!(
                    formatter,
                    "page size {requested} exceeds the maximum of {maximum}"
                )
            }
            Self::EmptyCursor => formatter.write_str("keyset cursor must not be empty"),
        }
    }
}

impl Error for PaginationError {}

/// A bounded, non-zero number of results requested for one search page.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PageSize(u32);

impl PageSize {
    /// The default local-search page size.
    pub const DEFAULT: Self = Self(DEFAULT_PAGE_SIZE);

    /// The largest valid local-search page size.
    pub const MAX: Self = Self(MAX_PAGE_SIZE);

    /// Creates a validated page size.
    ///
    /// # Errors
    ///
    /// Returns [`PaginationError::ZeroPageSize`] when `value` is zero, or
    /// [`PaginationError::PageSizeExceedsMaximum`] when it exceeds
    /// [`MAX_PAGE_SIZE`].
    pub fn new(value: u32) -> Result<Self, PaginationError> {
        if value == 0 {
            return Err(PaginationError::ZeroPageSize);
        }
        if value > MAX_PAGE_SIZE {
            return Err(PaginationError::PageSizeExceedsMaximum {
                requested: value,
                maximum: MAX_PAGE_SIZE,
            });
        }

        Ok(Self(value))
    }

    /// Returns the validated page size.
    #[must_use]
    pub const fn get(self) -> u32 {
        self.0
    }
}

impl Default for PageSize {
    fn default() -> Self {
        Self::DEFAULT
    }
}

impl TryFrom<u32> for PageSize {
    type Error = PaginationError;

    /// # Errors
    ///
    /// Returns [`PaginationError::ZeroPageSize`] when `value` is zero, or
    /// [`PaginationError::PageSizeExceedsMaximum`] when it exceeds
    /// [`MAX_PAGE_SIZE`].
    fn try_from(value: u32) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

/// An opaque continuation token for a keyset-ordered search result page.
///
/// The token is deliberately not parsed, normalized, or interpreted by this
/// value type. Callers must use a cursor returned by the same search ordering.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct KeysetCursor(String);

impl KeysetCursor {
    /// Creates a non-empty opaque keyset cursor.
    ///
    /// # Errors
    ///
    /// Returns [`PaginationError::EmptyCursor`] when `value` has no
    /// non-whitespace characters.
    pub fn new(value: impl Into<String>) -> Result<Self, PaginationError> {
        let value = value.into();
        if value.trim().is_empty() {
            return Err(PaginationError::EmptyCursor);
        }

        Ok(Self(value))
    }

    /// Returns the opaque cursor without interpreting it.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for KeysetCursor {
    type Error = PaginationError;

    /// # Errors
    ///
    /// Returns [`PaginationError::EmptyCursor`] when `value` has no
    /// non-whitespace characters.
    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl TryFrom<&str> for KeysetCursor {
    type Error = PaginationError;

    /// # Errors
    ///
    /// Returns [`PaginationError::EmptyCursor`] when `value` has no
    /// non-whitespace characters.
    fn try_from(value: &str) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

/// A bounded local-search page request with an optional keyset continuation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchPageRequest {
    page_size: PageSize,
    cursor: Option<KeysetCursor>,
}

impl SearchPageRequest {
    /// Creates a request from already validated pagination values.
    #[must_use]
    pub fn new(page_size: PageSize, cursor: Option<KeysetCursor>) -> Self {
        Self { page_size, cursor }
    }

    /// Returns the requested, bounded result count.
    #[must_use]
    pub const fn page_size(&self) -> PageSize {
        self.page_size
    }

    /// Returns the continuation cursor, when requesting a later page.
    #[must_use]
    pub fn cursor(&self) -> Option<&KeysetCursor> {
        self.cursor.as_ref()
    }
}

impl Default for SearchPageRequest {
    fn default() -> Self {
        Self::new(PageSize::default(), None)
    }
}

#[cfg(test)]
mod tests {
    use super::{
        DEFAULT_PAGE_SIZE, KeysetCursor, MAX_PAGE_SIZE, PageSize, PaginationError,
        SearchPageRequest,
    };

    #[test]
    fn page_size_defaults_to_fifty() {
        assert_eq!(PageSize::default().get(), DEFAULT_PAGE_SIZE);
        assert_eq!(SearchPageRequest::default().page_size().get(), 50);
    }

    #[test]
    fn page_size_accepts_its_smallest_and_largest_valid_values() {
        let smallest = PageSize::new(1).expect("valid smallest page size");
        let largest = PageSize::new(MAX_PAGE_SIZE).expect("valid largest page size");

        assert_eq!(smallest.get(), 1);
        assert_eq!(largest, PageSize::MAX);
    }

    #[test]
    fn page_size_rejects_zero() {
        assert_eq!(PageSize::new(0), Err(PaginationError::ZeroPageSize));
    }

    #[test]
    fn page_size_rejects_values_above_the_maximum() {
        assert_eq!(
            PageSize::new(MAX_PAGE_SIZE + 1),
            Err(PaginationError::PageSizeExceedsMaximum {
                requested: MAX_PAGE_SIZE + 1,
                maximum: MAX_PAGE_SIZE,
            })
        );
    }

    #[test]
    fn cursor_rejects_empty_or_whitespace_only_values() {
        assert_eq!(KeysetCursor::new(""), Err(PaginationError::EmptyCursor));
        assert_eq!(
            KeysetCursor::new(" \t\n "),
            Err(PaginationError::EmptyCursor)
        );
    }

    #[test]
    fn cursor_preserves_a_non_empty_opaque_value() {
        let cursor = KeysetCursor::new("  cursor:opaque-value  ").expect("valid cursor");

        assert_eq!(cursor.as_str(), "  cursor:opaque-value  ");
    }

    #[test]
    fn request_keeps_its_validated_page_size_and_optional_cursor() {
        let page_size = PageSize::new(75).expect("valid page size");
        let cursor = KeysetCursor::new("next-page").expect("valid cursor");
        let request = SearchPageRequest::new(page_size, Some(cursor));

        assert_eq!(request.page_size(), page_size);
        assert_eq!(
            request.cursor().map(KeysetCursor::as_str),
            Some("next-page")
        );
    }

    #[test]
    fn default_request_has_no_continuation_cursor() {
        assert_eq!(SearchPageRequest::default().cursor(), None);
    }
}
