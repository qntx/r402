//! Error types for the NEAR `"exact"` payment scheme.

use compact_str::CompactString;
use r402_core::error::ErrorReason;
use r402_core::wire::VerifyResponse;

/// Errors specific to NEAR exact scheme operations.
#[derive(Debug, thiserror::Error)]
pub enum NearExactError {
    /// JSON-RPC or chain-state read failed.
    #[error("near rpc: {0}")]
    Rpc(String),
    /// Borsh or base64 decoding failed.
    #[error("decode error: {0}")]
    Decode(String),
    /// Client signing failed.
    #[error("signing error: {0}")]
    Signing(String),
}

/// A verify failure that maps 1:1 onto a TS `invalidReason` string.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NearInvalid {
    /// Wire `invalidReason`.
    pub reason: ErrorReason,
    /// Payer account, attached only after signature verification.
    pub payer: Option<CompactString>,
}

impl NearInvalid {
    /// Constructs an invalid outcome with no payer.
    #[must_use]
    pub fn new(reason: &'static str) -> Self {
        Self {
            reason: ErrorReason::from_wire(reason),
            payer: None,
        }
    }

    /// Attaches a cryptographically attributed payer.
    #[must_use]
    pub fn with_payer(mut self, payer: impl Into<CompactString>) -> Self {
        self.payer = Some(payer.into());
        self
    }

    /// Converts to a wire [`VerifyResponse`].
    #[must_use]
    pub fn into_response(self) -> VerifyResponse {
        VerifyResponse::invalid(self.payer, self.reason)
    }
}

/// Convenience: builds [`NearInvalid`] from a static reason code.
#[must_use]
pub fn invalid(reason: &'static str) -> NearInvalid {
    NearInvalid::new(reason)
}
