//! Error types for MCP x402 payment integration.
//!
//! This module defines [`McpPaymentError`] for all failure modes during
//! MCP payment flows, and [`PaymentRequiredError`] for representing
//! 402 payment required responses as typed errors.

use r402::proto;

/// Errors that can occur during MCP x402 payment operations.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum McpPaymentError {
    /// The tool call itself failed (non-payment error).
    #[error("Tool call failed: {0}")]
    ToolCallFailed(String),

    /// No payment option matched the client's capabilities.
    #[error("No matching payment option found")]
    NoMatchingPaymentOption,

    /// Failed to create a payment payload.
    #[error("Failed to create payment: {0}")]
    PaymentCreationFailed(String),

    /// Payment signing failed.
    #[error("Failed to sign payment: {0}")]
    SigningFailed(String),

    /// Payment verification failed on the server side.
    #[error("Payment verification failed: {0}")]
    VerificationFailed(String),

    /// Payment settlement failed on the server side.
    #[error("Settlement failed: {0}")]
    SettlementFailed(String),

    /// A lifecycle hook aborted the operation.
    #[error("Operation aborted: {0}")]
    Aborted(String),

    /// Payment is required but auto-payment is disabled or denied.
    #[error("Payment required")]
    PaymentRequired(Box<PaymentRequiredError>),

    /// JSON serialization/deserialization error.
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    /// An error from the facilitator layer.
    #[error(transparent)]
    Facilitator(#[from] r402::facilitator::FacilitatorError),

    /// Client-side scheme error.
    #[error("Client error: {0}")]
    Client(#[from] r402::scheme::ClientError),
}

/// Represents a 402 payment required response from an MCP tool call.
///
/// This error carries the full [`proto::PaymentRequired`] data so callers
/// can inspect the accepted payment methods and potentially retry with payment.
#[derive(Debug, Clone)]
pub struct PaymentRequiredError {
    /// Human-readable error message.
    pub message: String,
    /// The payment required data from the tool response.
    pub payment_required: proto::PaymentRequired,
}

impl std::fmt::Display for PaymentRequiredError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for PaymentRequiredError {}

impl PaymentRequiredError {
    /// Creates a new payment required error.
    #[must_use]
    pub fn new(message: impl Into<String>, payment_required: proto::PaymentRequired) -> Self {
        Self {
            message: message.into(),
            payment_required,
        }
    }
}

/// Returns `true` if the error is a [`PaymentRequiredError`].
#[must_use]
pub const fn is_payment_required_error(err: &McpPaymentError) -> bool {
    matches!(err, McpPaymentError::PaymentRequired(_))
}
