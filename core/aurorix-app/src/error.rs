//! Application error values with stable machine-readable codes.
//!
//! The application layer exposes only fixed, display-safe messages. Detailed
//! causes remain inside the owning implementation instead of becoming part of
//! a public error value.

use std::{error::Error, fmt};

/// A correlation identifier for one application request.
///
/// This value is intentionally opaque to the application layer. It identifies
/// one request for diagnostics, but it is not a durable operation identifier
/// or an idempotency key.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RequestCorrelationId(String);

impl RequestCorrelationId {
    /// Creates an opaque request correlation identifier.
    ///
    /// # Errors
    ///
    /// Returns [`RequestCorrelationIdError::Empty`] when `value` contains no
    /// non-whitespace characters.
    pub fn new(value: impl Into<String>) -> Result<Self, RequestCorrelationIdError> {
        let value = value.into();
        if value.trim().is_empty() {
            return Err(RequestCorrelationIdError::Empty);
        }

        Ok(Self(value))
    }

    /// Returns the opaque correlation identifier.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for RequestCorrelationId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("RequestCorrelationId(<redacted>)")
    }
}

impl TryFrom<&str> for RequestCorrelationId {
    type Error = RequestCorrelationIdError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl TryFrom<String> for RequestCorrelationId {
    type Error = RequestCorrelationIdError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

/// Validation failure for a [`RequestCorrelationId`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RequestCorrelationIdError {
    /// The supplied identifier has no non-whitespace characters.
    Empty,
}

impl fmt::Display for RequestCorrelationIdError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => formatter.write_str("request correlation ID must not be empty"),
        }
    }
}

impl Error for RequestCorrelationIdError {}

/// A stable machine-readable code for an application failure.
///
/// Callers branch on [`AppErrorCode`] values or [`Self::as_str`], never on a
/// display message. The `app` namespace is reserved for application-layer
/// failures; transport and infrastructure contracts define their own codes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AppErrorCode {
    /// The request could not be understood or was structurally invalid.
    InvalidRequest,
    /// The request is valid, but cannot run in the current application state.
    InvalidState,
    /// The requested application resource does not exist.
    NotFound,
    /// The request conflicts with the current application state.
    Conflict,
    /// The application is temporarily unable to complete the request.
    Unavailable,
    /// The request may have changed durable state, but its outcome is unknown.
    OutcomeUnknown,
    /// The request requires a contract version that this application cannot use.
    IncompatibleVersion,
    /// The application could not complete the request for an internal reason.
    Internal,
}

impl AppErrorCode {
    /// Returns the stable namespaced code string.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::InvalidRequest => "app.invalid_request",
            Self::InvalidState => "app.invalid_state",
            Self::NotFound => "app.not_found",
            Self::Conflict => "app.conflict",
            Self::Unavailable => "app.unavailable",
            Self::OutcomeUnknown => "app.outcome_unknown",
            Self::IncompatibleVersion => "app.incompatible_version",
            Self::Internal => "app.internal",
        }
    }

    /// Returns the display-safe message associated with this code.
    #[must_use]
    pub const fn safe_message(self) -> SafeDisplayMessage {
        match self {
            Self::InvalidRequest => SafeDisplayMessage::InvalidRequest,
            Self::InvalidState => SafeDisplayMessage::InvalidState,
            Self::NotFound => SafeDisplayMessage::NotFound,
            Self::Conflict => SafeDisplayMessage::Conflict,
            Self::Unavailable => SafeDisplayMessage::Unavailable,
            Self::OutcomeUnknown => SafeDisplayMessage::OutcomeUnknown,
            Self::IncompatibleVersion => SafeDisplayMessage::IncompatibleVersion,
            Self::Internal => SafeDisplayMessage::Internal,
        }
    }

    /// Returns whether callers may retry the same request without first resolving state.
    #[must_use]
    pub const fn is_retryable(self) -> bool {
        matches!(self, Self::Unavailable)
    }
}

impl fmt::Display for AppErrorCode {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// A fixed message that is safe to present to an end user.
///
/// The message intentionally excludes implementation causes, upstream bodies,
/// local paths, and request identifiers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SafeDisplayMessage {
    /// The request is invalid.
    InvalidRequest,
    /// The request cannot run in the current state.
    InvalidState,
    /// The requested item was not found.
    NotFound,
    /// The request conflicts with the current state.
    Conflict,
    /// The request cannot be completed right now.
    Unavailable,
    /// The request outcome must be checked before another attempt.
    OutcomeUnknown,
    /// The application cannot use the required version.
    IncompatibleVersion,
    /// The request could not be completed.
    Internal,
}

impl SafeDisplayMessage {
    /// Returns the display-safe message text.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::InvalidRequest => "The request is invalid.",
            Self::InvalidState => "The request cannot run in the current state.",
            Self::NotFound => "The requested item was not found.",
            Self::Conflict => "The request conflicts with the current state.",
            Self::Unavailable => "The request cannot be completed right now.",
            Self::OutcomeUnknown => "The request outcome must be checked before trying again.",
            Self::IncompatibleVersion => "The required version is not supported.",
            Self::Internal => "The request could not be completed.",
        }
    }
}

impl fmt::Display for SafeDisplayMessage {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// An application failure with a stable code and request correlation identifier.
///
/// The safe display message and retryability are derived from the code, so they
/// cannot disagree with it or contain untrusted implementation detail.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppError {
    code: AppErrorCode,
    request_id: RequestCorrelationId,
}

impl AppError {
    /// Creates an application error for a correlated request.
    #[must_use]
    pub const fn new(code: AppErrorCode, request_id: RequestCorrelationId) -> Self {
        Self { code, request_id }
    }

    /// Returns the stable machine-readable failure code.
    #[must_use]
    pub const fn code(&self) -> AppErrorCode {
        self.code
    }

    /// Returns the identifier that correlates the failed request.
    #[must_use]
    pub fn request_id(&self) -> &RequestCorrelationId {
        &self.request_id
    }

    /// Returns the fixed display-safe message for this error.
    #[must_use]
    pub const fn safe_message(&self) -> SafeDisplayMessage {
        self.code.safe_message()
    }

    /// Returns whether callers may retry the same request without first resolving state.
    #[must_use]
    pub const fn is_retryable(&self) -> bool {
        self.code.is_retryable()
    }
}

impl fmt::Display for AppError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.safe_message().fmt(formatter)
    }
}

impl Error for AppError {}

#[cfg(test)]
mod tests {
    use super::{AppError, AppErrorCode, RequestCorrelationId, RequestCorrelationIdError};

    #[test]
    fn codes_have_stable_namespaces_messages_and_retryability() {
        let expected = [
            (
                AppErrorCode::InvalidRequest,
                "app.invalid_request",
                "The request is invalid.",
                false,
            ),
            (
                AppErrorCode::InvalidState,
                "app.invalid_state",
                "The request cannot run in the current state.",
                false,
            ),
            (
                AppErrorCode::NotFound,
                "app.not_found",
                "The requested item was not found.",
                false,
            ),
            (
                AppErrorCode::Conflict,
                "app.conflict",
                "The request conflicts with the current state.",
                false,
            ),
            (
                AppErrorCode::Unavailable,
                "app.unavailable",
                "The request cannot be completed right now.",
                true,
            ),
            (
                AppErrorCode::OutcomeUnknown,
                "app.outcome_unknown",
                "The request outcome must be checked before trying again.",
                false,
            ),
            (
                AppErrorCode::IncompatibleVersion,
                "app.incompatible_version",
                "The required version is not supported.",
                false,
            ),
            (
                AppErrorCode::Internal,
                "app.internal",
                "The request could not be completed.",
                false,
            ),
        ];

        for (code, expected_code, expected_message, expected_retryable) in expected {
            assert_eq!(code.as_str(), expected_code);
            assert_eq!(code.safe_message().as_str(), expected_message);
            assert_eq!(code.is_retryable(), expected_retryable);
        }
    }

    #[test]
    fn correlation_id_rejects_blank_values_without_normalizing_nonblank_ids() {
        assert_eq!(
            RequestCorrelationId::new(" \t\n "),
            Err(RequestCorrelationIdError::Empty)
        );

        let identifier = RequestCorrelationId::new(" request-42 ").expect("nonblank identifier");
        assert_eq!(identifier.as_str(), " request-42 ");
        assert_eq!(
            format!("{identifier:?}"),
            "RequestCorrelationId(<redacted>)"
        );
    }

    #[test]
    fn application_error_exposes_only_code_derived_public_values() {
        let request_id = RequestCorrelationId::new("request-42").expect("valid request ID");
        let error = AppError::new(AppErrorCode::OutcomeUnknown, request_id);

        assert_eq!(error.code(), AppErrorCode::OutcomeUnknown);
        assert_eq!(error.request_id().as_str(), "request-42");
        assert_eq!(
            error.safe_message().as_str(),
            "The request outcome must be checked before trying again."
        );
        assert!(!error.is_retryable());
        assert_eq!(error.to_string(), error.safe_message().as_str());
        assert!(!error.to_string().contains(error.request_id().as_str()));
    }
}
