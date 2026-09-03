//! Error types for the Keeta `"exact"` payment scheme.

use compact_str::CompactString;
use r402_protocol::error::ErrorReason;
use r402_protocol::payment::VerifyResponse;

/// A verify failure carrying a wire `invalidReason` and optional payer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeetaInvalid {
    /// Wire `invalidReason`.
    pub reason: ErrorReason,
    /// Payer account, attached after the block is decoded.
    pub payer: Option<CompactString>,
}

impl KeetaInvalid {
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

/// Convenience: builds [`KeetaInvalid`] from a static reason code.
#[must_use]
pub fn invalid(reason: &'static str) -> KeetaInvalid {
    KeetaInvalid::new(reason)
}
