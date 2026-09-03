//! Seller-side payment-requirements container.

use super::PaymentRequirements;

/// Seller-side wrapper around [`PaymentRequirements`].
///
/// Scheme-specific `extra` is filled during 402 construction, not on this type.
#[derive(Debug, Clone)]
pub struct PriceTag {
    /// Requirements being advertised.
    pub requirements: PaymentRequirements,
}

impl PriceTag {
    /// Wraps existing requirements.
    #[must_use]
    pub const fn new(requirements: PaymentRequirements) -> Self {
        Self { requirements }
    }

    /// Overrides `maxTimeoutSeconds`.
    #[must_use]
    pub const fn with_timeout(mut self, seconds: u64) -> Self {
        self.requirements.max_timeout_seconds = seconds;
        self
    }
}
