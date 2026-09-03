//! Error types for the Algorand `"exact"` payment scheme.

use compact_str::CompactString;
use r402_protocol::error::ErrorReason;
use r402_protocol::payment::VerifyResponse;

/// A verify failure carrying a wire `invalidReason` and optional payer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AlgorandInvalid {
    /// Wire `invalidReason`.
    pub reason: ErrorReason,
    /// Optional human-readable message.
    pub message: Option<CompactString>,
    /// Payer address, attached once the payment sender is known.
    pub payer: Option<CompactString>,
}

impl AlgorandInvalid {
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

    /// Attaches the payment sender.
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

/// Convenience: builds [`AlgorandInvalid`] from a static reason code.
#[must_use]
pub fn invalid(reason: &'static str) -> AlgorandInvalid {
    AlgorandInvalid::new(reason)
}
