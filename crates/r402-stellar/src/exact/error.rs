//! Error types for the Stellar `"exact"` payment scheme.

use compact_str::CompactString;
use r402_protocol::error::ErrorReason;
use r402_protocol::payment::VerifyResponse;

/// A verify failure carrying a wire `invalidReason` and optional payer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StellarInvalid {
    /// Wire `invalidReason`.
    pub reason: ErrorReason,
    /// Payer address, attached after transfer `from` is parsed.
    pub payer: Option<CompactString>,
    /// Optional human-readable detail (fee-ceiling messages).
    pub message: Option<CompactString>,
}

impl StellarInvalid {
    /// Constructs an invalid outcome with no payer.
    #[must_use]
    pub fn new(reason: &'static str) -> Self {
        Self {
            reason: ErrorReason::from_wire(reason),
            payer: None,
            message: None,
        }
    }

    /// Attaches a parsed payer address.
    #[must_use]
    pub fn with_payer(mut self, payer: impl Into<CompactString>) -> Self {
        self.payer = Some(payer.into());
        self
    }

    /// Attaches a human-readable invalid message.
    #[must_use]
    pub fn with_message(mut self, message: impl Into<CompactString>) -> Self {
        self.message = Some(message.into());
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

/// Convenience: builds [`StellarInvalid`] from a static reason code.
#[must_use]
pub fn invalid(reason: &'static str) -> StellarInvalid {
    StellarInvalid::new(reason)
}
