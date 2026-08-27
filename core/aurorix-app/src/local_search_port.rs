//! Application port for querying the local music catalog.
//!
//! The port is deliberately transport- and storage-neutral. Implementations
//! may use `SQLite` or another local index, but that choice stays outside this
//! crate. The application receives validated request values and an already
//! ordered result page; it does not own query syntax, ranking, or persistence.

use crate::{
    error::{AppError, RequestCorrelationId},
    search_projection::LocalSearchPage,
    search_query::LocalSearchRequest,
};

/// Application boundary for local catalog search.
///
/// A local-search implementation is responsible for executing the validated
/// request and returning a page in its established, deterministic order. The
/// continuation cursor is opaque to this port and must only be interpreted by
/// the implementation that produced it.
///
/// The request correlation identifier is supplied separately so failures can
/// return an [`AppError`] without exposing implementation details. Successful
/// results do not contain the identifier.
pub trait LocalSearchRepository: Send + Sync {
    /// Executes one bounded local-catalog search page.
    ///
    /// Implementations must keep the operation local-first: Provider
    /// availability, network state, and remote catalog results are outside
    /// this contract. A failure should use the supplied correlation identifier
    /// when constructing its [`AppError`].
    ///
    /// # Errors
    ///
    /// Returns an [`AppError`] with a stable application code and the supplied
    /// request correlation identifier when the page cannot be produced.
    fn search(
        &self,
        request: &LocalSearchRequest,
        request_id: &RequestCorrelationId,
    ) -> Result<LocalSearchPage, AppError>;
}

/// Alias emphasizing that [`LocalSearchRepository`] is an application port.
pub use LocalSearchRepository as LocalSearchPort;

#[cfg(test)]
mod tests {
    use super::{LocalSearchPage, LocalSearchPort};
    use crate::{
        error::{AppError, AppErrorCode, RequestCorrelationId},
        pagination::SearchPageRequest,
        search_projection::{LocalSearchAvailability, LocalSearchHit, LocalSearchResultKind},
        search_query::{LocalSearchRequest, SearchText},
    };
    use aurorix_model::ids::LocalCatalogEntityId;

    struct FixtureRepository {
        page: LocalSearchPage,
        failure: Option<AppErrorCode>,
    }

    impl LocalSearchPort for FixtureRepository {
        fn search(
            &self,
            request: &LocalSearchRequest,
            request_id: &RequestCorrelationId,
        ) -> Result<LocalSearchPage, AppError> {
            assert_eq!(request.search_text().as_str(), "Boards of Canada");
            assert_eq!(request.page(), &SearchPageRequest::default());

            self.failure.map_or_else(
                || Ok(self.page.clone()),
                |code| Err(AppError::new(code, request_id.clone())),
            )
        }
    }

    fn request() -> LocalSearchRequest {
        LocalSearchRequest::new(
            SearchText::new("Boards of Canada").expect("valid search text"),
            SearchPageRequest::default(),
        )
    }

    fn page() -> LocalSearchPage {
        let hit = LocalSearchHit::new(
            LocalSearchResultKind::Recording,
            LocalCatalogEntityId::new_v7(),
            "Dayvan Cowboy",
            vec!["Boards of Canada".to_owned()],
            None,
            None,
            LocalSearchAvailability::Available,
        )
        .expect("valid search hit");

        LocalSearchPage::new(vec![hit], None)
    }

    #[test]
    fn repository_returns_a_page_for_a_valid_request() {
        let repository = FixtureRepository {
            page: page(),
            failure: None,
        };
        let request_id = RequestCorrelationId::new("request-1").expect("valid request ID");

        let result = repository
            .search(&request(), &request_id)
            .expect("search succeeds");

        assert_eq!(result.hits().len(), 1);
        assert_eq!(result.hits()[0].title(), "Dayvan Cowboy");
        assert_eq!(result.next_cursor(), None);
    }

    #[test]
    fn repository_preserves_the_request_id_on_application_errors() {
        let repository = FixtureRepository {
            page: page(),
            failure: Some(AppErrorCode::Unavailable),
        };
        let request_id = RequestCorrelationId::new("request-2").expect("valid request ID");

        let error = repository
            .search(&request(), &request_id)
            .expect_err("search should fail");

        assert_eq!(error.code(), AppErrorCode::Unavailable);
        assert_eq!(error.request_id().as_str(), "request-2");
        assert!(error.is_retryable());
    }
}
