//! Error types for the x402 payment protocol.
//!
//! Corresponds to Python SDK's `schemas/errors.py`.

use std::fmt;

/// Base error type for x402 payment operations.
#[derive(Debug, thiserror::Error)]
pub enum PaymentError {
    /// Error during payment verification.
    #[error("{0}")]
    Verify(#[from] VerifyError),

    /// Error during payment settlement.
    #[error("{0}")]
    Settle(#[from] SettleError),

    /// No registered scheme found for scheme/network combination.
    #[error("{0}")]
    SchemeNotFound(#[from] SchemeNotFoundError),

    /// No payment requirements match registered schemes.
    #[error("{0}")]
    NoMatchingRequirements(#[from] NoMatchingRequirementsError),

    /// Payment was aborted by a before hook.
    #[error("{0}")]
    Aborted(#[from] PaymentAbortedError),
}

/// Error during payment verification.
#[derive(Debug, Clone)]
pub struct VerifyError {
    /// Machine-readable reason for the error.
    pub invalid_reason: String,
    /// Human-readable message for the error.
    pub invalid_message: Option<String>,
    /// The payer's address (if known).
    pub payer: Option<String>,
}

impl VerifyError {
    /// Creates a new verification error.
    #[must_use]
    pub fn new(reason: impl Into<String>) -> Self {
        Self {
            invalid_reason: reason.into(),
            invalid_message: None,
            payer: None,
        }
    }

    /// Sets the human-readable message.
    #[must_use]
    pub fn with_message(mut self, message: impl Into<String>) -> Self {
        self.invalid_message = Some(message.into());
        self
    }

    /// Sets the payer address.
    #[must_use]
    pub fn with_payer(mut self, payer: impl Into<String>) -> Self {
        self.payer = Some(payer.into());
        self
    }
}

impl fmt::Display for VerifyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(msg) = &self.invalid_message {
            write!(f, "{}: {}", self.invalid_reason, msg)
        } else {
            write!(f, "{}", self.invalid_reason)
        }
    }
}

impl std::error::Error for VerifyError {}

/// Error during payment settlement.
#[derive(Debug, Clone)]
pub struct SettleError {
    /// Machine-readable reason for the error.
    pub error_reason: String,
    /// Human-readable message for the error.
    pub error_message: Option<String>,
    /// Transaction hash/identifier (if available).
    pub transaction: Option<String>,
    /// The payer's address (if known).
    pub payer: Option<String>,
}

impl SettleError {
    /// Creates a new settlement error.
    #[must_use]
    pub fn new(reason: impl Into<String>) -> Self {
        Self {
            error_reason: reason.into(),
            error_message: None,
            transaction: None,
            payer: None,
        }
    }

    /// Sets the human-readable message.
    #[must_use]
    pub fn with_message(mut self, message: impl Into<String>) -> Self {
        self.error_message = Some(message.into());
        self
    }

    /// Sets the transaction hash.
    #[must_use]
    pub fn with_transaction(mut self, tx: impl Into<String>) -> Self {
        self.transaction = Some(tx.into());
        self
    }

    /// Sets the payer address.
    #[must_use]
    pub fn with_payer(mut self, payer: impl Into<String>) -> Self {
        self.payer = Some(payer.into());
        self
    }
}

impl fmt::Display for SettleError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(msg) = &self.error_message {
            write!(f, "{}: {}", self.error_reason, msg)
        } else {
            write!(f, "{}", self.error_reason)
        }
    }
}

impl std::error::Error for SettleError {}

/// No registered scheme found for scheme/network combination.
#[derive(Debug, Clone)]
pub struct SchemeNotFoundError {
    /// The requested scheme.
    pub scheme: String,
    /// The requested network.
    pub network: String,
}

impl SchemeNotFoundError {
    /// Creates a new scheme-not-found error.
    #[must_use]
    pub fn new(scheme: impl Into<String>, network: impl Into<String>) -> Self {
        Self {
            scheme: scheme.into(),
            network: network.into(),
        }
    }
}

impl fmt::Display for SchemeNotFoundError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "No scheme '{}' registered for network '{}'",
            self.scheme, self.network
        )
    }
}

impl std::error::Error for SchemeNotFoundError {}

/// No payment requirements match registered schemes.
#[derive(Debug, Clone)]
pub struct NoMatchingRequirementsError {
    /// Reason for the error.
    pub reason: String,
}

impl NoMatchingRequirementsError {
    /// Creates a new no-matching-requirements error.
    #[must_use]
    pub fn new(reason: impl Into<String>) -> Self {
        Self {
            reason: reason.into(),
        }
    }
}

impl fmt::Display for NoMatchingRequirementsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.reason)
    }
}

impl std::error::Error for NoMatchingRequirementsError {}

/// Payment was aborted by a before hook.
#[derive(Debug, Clone)]
pub struct PaymentAbortedError {
    /// The reason for aborting.
    pub reason: String,
}

impl PaymentAbortedError {
    /// Creates a new payment-aborted error.
    #[must_use]
    pub fn new(reason: impl Into<String>) -> Self {
        Self {
            reason: reason.into(),
        }
    }
}

impl fmt::Display for PaymentAbortedError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Payment aborted: {}", self.reason)
    }
}

impl std::error::Error for PaymentAbortedError {}
