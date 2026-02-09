//! Error types for the HTTP transport layer.

use r402::ProtocolError;

/// Errors that can occur during HTTP header encoding/decoding.
#[derive(Debug, thiserror::Error)]
pub enum HttpError {
    /// JSON serialization/deserialization failed.
    #[error("JSON error: {0}")]
    Serialize(#[from] serde_json::Error),

    /// Base64 decoding failed.
    #[error("Base64 decode error: {0}")]
    Base64(#[from] base64::DecodeError),

    /// Protocol-level error (version detection, missing fields, etc.).
    #[error("protocol error: {0}")]
    Protocol(#[source] ProtocolError),
}
