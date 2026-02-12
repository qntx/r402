//! Error types for x402 payment verification.
//!
//! This module defines structured error types used when payment verification
//! or settlement fails, along with machine-readable reason codes.

use serde::{Deserialize, Serialize};

/// Errors that can occur during payment verification.
///
/// These errors are returned when a payment fails validation checks
/// performed by the facilitator before settlement.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum PaymentVerificationError {
    /// The payment payload format is invalid or malformed.
    #[error("Invalid format: {0}")]
    InvalidFormat(String),
    /// The payment amount doesn't match the requirements.
    #[error("Payment amount is invalid with respect to the payment requirements")]
    InvalidPaymentAmount,
    /// The payment authorization's `validAfter` timestamp is in the future.
    #[error("Payment authorization is not yet valid")]
    Early,
    /// The payment authorization's `validBefore` timestamp has passed.
    #[error("Payment authorization is expired")]
    Expired,
    /// The payment's chain ID doesn't match the requirements.
    #[error("Payment chain id is invalid with respect to the payment requirements")]
    ChainIdMismatch,
    /// The payment recipient doesn't match the requirements.
    #[error("Payment recipient is invalid with respect to the payment requirements")]
    RecipientMismatch,
    /// The payment asset (token) doesn't match the requirements.
    #[error("Payment asset is invalid with respect to the payment requirements")]
    AssetMismatch,
    /// The payer's on-chain balance is insufficient.
    #[error("Onchain balance is not enough to cover the payment amount")]
    InsufficientFunds,
    /// The payer's Permit2 allowance is insufficient.
    #[error("Permit2 allowance is not enough to cover the payment amount")]
    Permit2AllowanceInsufficient,
    /// The payment signature is invalid.
    #[error("{0}")]
    InvalidSignature(String),
    /// Transaction simulation failed.
    #[error("{0}")]
    TransactionSimulation(String),
    /// The chain is not supported by this facilitator.
    #[error("Unsupported chain")]
    UnsupportedChain,
    /// The payment scheme is not supported by this facilitator.
    #[error("Unsupported scheme")]
    UnsupportedScheme,
    /// The accepted payment details don't match the requirements.
    #[error("Accepted does not match payment requirements")]
    AcceptedRequirementsMismatch,
    /// The EIP-3009 authorization nonce has already been consumed on-chain.
    #[error("Authorization nonce already used")]
    NonceAlreadyUsed,
}

impl AsPaymentProblem for PaymentVerificationError {
    fn as_payment_problem(&self) -> PaymentProblem {
        let error_reason = match self {
            Self::InvalidFormat(_) => ErrorReason::InvalidFormat,
            Self::InvalidPaymentAmount => ErrorReason::InvalidPaymentAmount,
            Self::InsufficientFunds => ErrorReason::InsufficientFunds,
            Self::Permit2AllowanceInsufficient => ErrorReason::Permit2AllowanceInsufficient,
            Self::Early => ErrorReason::InvalidPaymentEarly,
            Self::Expired => ErrorReason::InvalidPaymentExpired,
            Self::ChainIdMismatch => ErrorReason::ChainIdMismatch,
            Self::RecipientMismatch => ErrorReason::RecipientMismatch,
            Self::AssetMismatch => ErrorReason::AssetMismatch,
            Self::InvalidSignature(_) => ErrorReason::InvalidSignature,
            Self::TransactionSimulation(_) => ErrorReason::TransactionSimulation,
            Self::UnsupportedChain => ErrorReason::UnsupportedChain,
            Self::UnsupportedScheme => ErrorReason::UnsupportedScheme,
            Self::AcceptedRequirementsMismatch => ErrorReason::AcceptedRequirementsMismatch,
            Self::NonceAlreadyUsed => ErrorReason::NonceAlreadyUsed,
        };
        PaymentProblem::new(error_reason, self.to_string())
    }
}

impl From<serde_json::Error> for PaymentVerificationError {
    fn from(value: serde_json::Error) -> Self {
        Self::InvalidFormat(value.to_string())
    }
}

/// Machine-readable error reason codes for payment failures.
///
/// These codes are used in error responses to allow clients to
/// programmatically handle different failure scenarios.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum ErrorReason {
    /// The payment payload format is invalid.
    InvalidFormat,
    /// The payment amount is incorrect.
    InvalidPaymentAmount,
    /// The payment authorization is not yet valid.
    InvalidPaymentEarly,
    /// The payment authorization has expired.
    InvalidPaymentExpired,
    /// The chain ID doesn't match.
    ChainIdMismatch,
    /// The recipient address doesn't match.
    RecipientMismatch,
    /// The token asset doesn't match.
    AssetMismatch,
    /// The accepted details don't match requirements.
    AcceptedRequirementsMismatch,
    /// The signature is invalid.
    InvalidSignature,
    /// Transaction simulation failed.
    TransactionSimulation,
    /// Insufficient on-chain balance.
    InsufficientFunds,
    /// Insufficient Permit2 allowance (payer needs to approve Permit2 contract).
    Permit2AllowanceInsufficient,
    /// The chain is not supported.
    UnsupportedChain,
    /// The scheme is not supported.
    UnsupportedScheme,
    /// The authorization nonce has already been used.
    NonceAlreadyUsed,
    /// An unexpected error occurred.
    UnexpectedError,
}

impl ErrorReason {
    /// Returns the `snake_case` string representation matching the wire format.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::InvalidFormat => "invalid_format",
            Self::InvalidPaymentAmount => "invalid_payment_amount",
            Self::InvalidPaymentEarly => "invalid_payment_early",
            Self::InvalidPaymentExpired => "invalid_payment_expired",
            Self::ChainIdMismatch => "chain_id_mismatch",
            Self::RecipientMismatch => "recipient_mismatch",
            Self::AssetMismatch => "asset_mismatch",
            Self::AcceptedRequirementsMismatch => "accepted_requirements_mismatch",
            Self::InvalidSignature => "invalid_signature",
            Self::TransactionSimulation => "transaction_simulation",
            Self::InsufficientFunds => "insufficient_funds",
            Self::Permit2AllowanceInsufficient => "permit2_allowance_insufficient",
            Self::UnsupportedChain => "unsupported_chain",
            Self::UnsupportedScheme => "unsupported_scheme",
            Self::NonceAlreadyUsed => "nonce_already_used",
            Self::UnexpectedError => "unexpected_error",
        }
    }
}

impl core::fmt::Display for ErrorReason {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Trait for converting errors into structured payment problems.
pub trait AsPaymentProblem {
    /// Converts this error into a [`PaymentProblem`].
    fn as_payment_problem(&self) -> PaymentProblem;
}

/// A structured payment error with reason code and details.
///
/// This type is used to return detailed error information to clients
/// when a payment fails verification or settlement.
#[derive(Debug)]
pub struct PaymentProblem {
    /// The machine-readable error reason.
    reason: ErrorReason,
    /// Human-readable error details.
    details: String,
}

impl PaymentProblem {
    /// Creates a new payment problem with the given reason and details.
    #[must_use]
    pub const fn new(reason: ErrorReason, details: String) -> Self {
        Self { reason, details }
    }

    /// Returns the error reason code.
    #[must_use]
    pub const fn reason(&self) -> ErrorReason {
        self.reason
    }

    /// Returns the human-readable error details.
    #[must_use]
    pub fn details(&self) -> &str {
        &self.details
    }
}
