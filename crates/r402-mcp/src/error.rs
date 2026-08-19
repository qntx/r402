//! Errors for MCP × x402 (Go `PaymentRequiredError` + client errors).

use r402_core::wire::PaymentRequired;

use crate::MCP_PAYMENT_REQUIRED_CODE;

/// Go `PaymentRequiredError` — JSON-RPC style code **402** with payment data.
#[derive(Debug, Clone)]
pub struct PaymentRequiredError {
    /// Always [`MCP_PAYMENT_REQUIRED_CODE`] (402).
    pub code: i32,
    /// Human-readable message.
    pub message: String,
    /// Payment requirements payload.
    pub payment_required: PaymentRequired,
}

impl PaymentRequiredError {
    /// Builds a 402 payment-required error.
    #[must_use]
    pub fn new(message: impl Into<String>, payment_required: PaymentRequired) -> Self {
        Self {
            code: MCP_PAYMENT_REQUIRED_CODE,
            message: message.into(),
            payment_required,
        }
    }
}

impl std::fmt::Display for PaymentRequiredError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for PaymentRequiredError {}

/// Client orchestration errors.
#[derive(Debug, thiserror::Error)]
pub enum McpClientError {
    /// Underlying MCP transport / tool call failed.
    #[error("mcp transport: {0}")]
    Transport(String),
    /// Building a payment payload failed.
    #[error("payment creation: {0}")]
    Payment(String),
    /// Server still required payment after a signed attempt.
    ///
    /// Carries the corrective challenge (when present) and whether
    /// `on_payment_response` requested recovery so callers can retry.
    #[error("payment still required after attempt")]
    StillRequired {
        /// Corrective `PaymentRequired` from the tool result, if parsed.
        payment_required: Option<Box<PaymentRequired>>,
        /// `true` when payment-response hooks signalled recovery.
        recovery_requested: bool,
    },
    /// Auto-payment disabled or user denied — includes 402 data when available.
    #[error(transparent)]
    PaymentRequired(#[from] Box<PaymentRequiredError>),
}

impl From<PaymentRequiredError> for McpClientError {
    fn from(value: PaymentRequiredError) -> Self {
        Self::PaymentRequired(Box::new(value))
    }
}
