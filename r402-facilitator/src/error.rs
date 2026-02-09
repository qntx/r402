//! Error types for the local facilitator service.

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};

/// Errors that can occur in the local facilitator service.
#[derive(Debug, thiserror::Error)]
pub enum FacilitatorError {
    /// No scheme handler found for the given scheme/network pair.
    #[error("{0}")]
    SchemeNotFound(#[from] r402::error::SchemeNotFoundError),

    /// JSON deserialization of request body failed.
    #[error("invalid request body: {0}")]
    InvalidBody(#[from] serde_json::Error),

    /// Protocol-level error (version detection, missing fields, etc.).
    #[error("protocol error: {0}")]
    Protocol(#[from] r402::ProtocolError),

    /// Scheme execution error (verify/settle failed).
    #[error("scheme error: {0}")]
    Scheme(Box<dyn std::error::Error + Send + Sync>),
}

impl FacilitatorError {
    /// Creates a scheme execution error from a boxed error.
    #[must_use]
    pub fn scheme(err: Box<dyn std::error::Error + Send + Sync>) -> Self {
        Self::Scheme(err)
    }
}

impl IntoResponse for FacilitatorError {
    fn into_response(self) -> Response {
        let status = match &self {
            Self::SchemeNotFound(_) => StatusCode::NOT_FOUND,
            Self::InvalidBody(_) | Self::Protocol(_) => StatusCode::BAD_REQUEST,
            Self::Scheme(_) => StatusCode::INTERNAL_SERVER_ERROR,
        };
        let body = serde_json::json!({ "error": self.to_string() });
        (status, axum::Json(body)).into_response()
    }
}
