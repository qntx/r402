//! Error types for the XRPL `"exact"` payment scheme.

use compact_str::CompactString;
use r402_core::error::ErrorReason;
use r402_core::wire::VerifyResponse;

/// A verify failure carrying a wire `invalidReason` and optional payer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct XrplInvalid {
    /// Wire `invalidReason`.
    pub reason: ErrorReason,
    /// Payer classic address, attached only after signature verification.
    pub payer: Option<CompactString>,
}

impl XrplInvalid {
    /// Constructs an invalid outcome with no payer.
    #[must_use]
    pub fn new(reason: impl Into<ErrorReason>) -> Self {
        Self {
            reason: reason.into(),
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

/// Convenience: builds [`XrplInvalid`] from a static reason code.
#[must_use]
pub fn invalid(reason: &'static str) -> XrplInvalid {
    XrplInvalid::new(ErrorReason::from_wire(reason))
}

/// Convenience: builds [`XrplInvalid`] from an owned reason string.
#[must_use]
pub fn invalid_owned(reason: impl Into<CompactString>) -> XrplInvalid {
    XrplInvalid::new(ErrorReason::from_wire(reason.into().as_str()))
}
