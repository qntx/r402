//! Seller-side builder for payment requirements.

use std::fmt::{self, Debug, Formatter};
use std::sync::Arc;

use super::{PaymentRequirements, SupportedResponse};

/// Type alias for enrichment callbacks.
///
/// An enricher receives mutable access to a [`PriceTag`] plus the
/// facilitator's [`SupportedResponse`] so it can fill in scheme-specific
/// `extra` fields (e.g. Solana fee payer, Permit2 nonce parameters).
pub type Enricher = Arc<dyn Fn(&mut PriceTag, &SupportedResponse) + Send + Sync>;

/// Mutable payment-requirements container with a lazy enrichment callback.
///
/// Sellers construct a `PriceTag` once from a price + chain + asset, then
/// the HTTP paygate calls [`Self::enrich`] with the facilitator's
/// capabilities to materialize chain-specific fields.
#[derive(Clone)]
pub struct PriceTag {
    /// The requirements being built.
    pub requirements: PaymentRequirements,
    /// Optional enricher invoked by [`Self::enrich`].
    #[doc(hidden)]
    pub enricher: Option<Enricher>,
}

impl PriceTag {
    /// Constructs a price tag around existing requirements (no enricher).
    #[must_use]
    pub const fn new(requirements: PaymentRequirements) -> Self {
        Self {
            requirements,
            enricher: None,
        }
    }

    /// Sets an enricher that runs on [`Self::enrich`].
    #[must_use]
    pub fn with_enricher(mut self, enricher: Enricher) -> Self {
        self.enricher = Some(enricher);
        self
    }

    /// Invokes the configured enricher, if any, with the facilitator's
    /// capability snapshot.
    pub fn enrich(&mut self, capabilities: &SupportedResponse) {
        if let Some(enricher) = self.enricher.clone() {
            enricher(self, capabilities);
        }
    }

    /// Overrides the `maxTimeoutSeconds` field.
    #[must_use]
    pub const fn with_timeout(mut self, seconds: u64) -> Self {
        self.requirements.max_timeout_seconds = seconds;
        self
    }
}

impl Debug for PriceTag {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_struct("PriceTag")
            .field("requirements", &self.requirements)
            .field("enricher", &self.enricher.as_ref().map(|_| "<fn>"))
            .finish()
    }
}

/// Matches a [`PriceTag`] against wire-level requirements on the five
/// protocol-critical fields only (scheme / network / amount / asset / pay_to).
///
/// `max_timeout_seconds` and `extra` are deliberately ignored so enriched
/// fields attached by the facilitator do not cause false negatives.
impl PartialEq<PaymentRequirements> for PriceTag {
    fn eq(&self, other: &PaymentRequirements) -> bool {
        let this = &self.requirements;
        this.scheme == other.scheme
            && this.network == other.network
            && this.amount == other.amount
            && this.asset == other.asset
            && this.pay_to == other.pay_to
    }
}
