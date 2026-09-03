//! Error types for the Aptos `"exact"` payment scheme.

use compact_str::CompactString;
use r402_protocol::error::ErrorReason;
use r402_protocol::payment::VerifyResponse;

/// A verify failure carrying a wire `invalidReason` and optional payer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AptosInvalid {
    /// Wire `invalidReason`.
    pub reason: ErrorReason,
    /// Payer account, attached after the sender is known.
    pub payer: Option<CompactString>,
}

impl AptosInvalid {
    /// Constructs an invalid outcome with no payer.
    #[must_use]
    pub fn new(reason: &'static str) -> Self {
        Self {
            reason: ErrorReason::from_wire(reason),
            payer: None,
        }
    }

    /// Constructs an invalid outcome from an owned reason string.
    #[must_use]
    pub fn from_owned(reason: impl AsRef<str>) -> Self {
        Self {
            reason: ErrorReason::from_wire(reason.as_ref()),
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

/// Convenience: builds [`AptosInvalid`] from a static reason code.
#[must_use]
pub fn invalid(reason: &'static str) -> AptosInvalid {
    AptosInvalid::new(reason)
}
