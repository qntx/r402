//! Error types for the x402 payment gate middleware.
//!
//! This module centralizes verification and settlement errors used across
//! the server-side payment gate components.

/// Common verification errors shared between protocol versions.
#[derive(Debug, thiserror::Error)]
pub enum VerificationError {
    /// Required payment header is missing.
    #[error("{0} header is required")]
    PaymentHeaderRequired(&'static str),
    /// Payment header is present but malformed.
    #[error("Invalid or malformed payment header")]
    InvalidPaymentHeader,
    /// No matching payment requirements found.
    #[error("Unable to find matching payment requirements")]
    NoPaymentMatching,
    /// Verification with facilitator failed.
    #[error("Verification failed: {0}")]
    VerificationFailed(String),
}

/// Paygate error type that wraps verification and settlement errors.
#[derive(Debug, thiserror::Error)]
pub enum PaygateError {
    /// Payment verification failed.
    #[error(transparent)]
    Verification(#[from] VerificationError),
    /// On-chain settlement failed.
    #[error("Settlement failed: {0}")]
    Settlement(String),
}
