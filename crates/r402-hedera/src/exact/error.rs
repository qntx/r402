//! Error types for the Hedera `"exact"` payment scheme.

use compact_str::CompactString;
use r402_core::error::ErrorReason;
use r402_core::wire::VerifyResponse;

/// A verify failure carrying a wire `invalidReason` and optional payer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HederaInvalid {
    /// Wire `invalidReason`.
    pub reason: ErrorReason,
    /// Optional human-readable detail.
    pub message: Option<CompactString>,
    /// Payer account, attached after it can be inferred.
    pub payer: Option<CompactString>,
}

impl HederaInvalid {
    /// Constructs an invalid outcome with no payer.
    #[must_use]
    pub fn new(reason: &'static str) -> Self {
        Self {
            reason: ErrorReason::from_wire(reason),
            message: None,
            payer: None,
        }
    }

    /// Attaches a human-readable message.
    #[must_use]
    pub fn with_message(mut self, message: impl Into<CompactString>) -> Self {
        self.message = Some(message.into());
        self
    }

    /// Attaches an inferred payer.
    #[must_use]
    pub fn with_payer(mut self, payer: impl Into<CompactString>) -> Self {
        self.payer = Some(payer.into());
        self
    }

    /// Converts to a wire [`VerifyResponse`].
    #[must_use]
    pub fn into_response(self) -> VerifyResponse {
        match self.message {
            Some(message) => VerifyResponse::invalid_with_message(self.payer, self.reason, message),
            None => VerifyResponse::invalid(self.payer, self.reason),
        }
    }
}

/// Convenience: builds [`HederaInvalid`] from a static reason code.
#[must_use]
pub fn invalid(reason: &'static str) -> HederaInvalid {
    HederaInvalid::new(reason)
}
