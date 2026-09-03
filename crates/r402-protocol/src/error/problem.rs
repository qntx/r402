//! Internal bridge from Rust errors to wire reason codes.

use super::ErrorReason;

/// Structured payment-problem record used at verify/settle boundaries.
///
/// Not itself serializable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaymentProblem {
    reason: ErrorReason,
    details: String,
}

impl PaymentProblem {
    /// Creates a new problem record.
    #[must_use]
    pub const fn new(reason: ErrorReason, details: String) -> Self {
        Self { reason, details }
    }

    /// Wire-level reason code.
    #[must_use]
    pub fn reason(&self) -> ErrorReason {
        self.reason.clone()
    }

    /// Borrowed wire-level reason code.
    #[must_use]
    pub const fn reason_ref(&self) -> &ErrorReason {
        &self.reason
    }

    /// Human-readable details.
    #[must_use]
    pub fn details(&self) -> &str {
        &self.details
    }
}

/// Converts an error into a [`PaymentProblem`].
pub trait AsPaymentProblem {
    /// Canonical wire-level payment problem.
    fn as_payment_problem(&self) -> PaymentProblem;
}
