//! Server-side scheme abstractions.
//!
//! A resource server (seller) uses a [`SchemeServer`] to translate
//! human-readable prices like `"$0.01"` into a full
//! [`PaymentRequirements`] with scheme-specific `extra` data filled in.

use compact_str::CompactString;

use super::Sealed;
use crate::chain::ChainId;
use crate::wire::PaymentRequirements;

/// A resolved asset amount ready for insertion into
/// [`PaymentRequirements`].
#[derive(Debug, Clone)]
pub struct AssetAmount {
    /// Token asset address / mint (wire-level string).
    pub asset: CompactString,
    /// Amount in the token's smallest unit, stringified.
    pub amount: CompactString,
}

/// Errors emitted by a [`SchemeServer`].
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum SchemeServerError {
    /// The price could not be parsed.
    #[error("invalid price: {0}")]
    InvalidPrice(String),
    /// The chain or asset is not configured.
    #[error("unsupported chain or asset: {0}")]
    UnsupportedChain(String),
    /// Any other server-side failure.
    #[error("{0}")]
    Other(String),
}

/// Server-side scheme interface.
///
/// Sealed: only crates inside this workspace may implement it.
pub trait SchemeServer: super::SchemeId + Sealed + Send + Sync {
    /// Parses a human-readable price into the scheme's internal amount form.
    ///
    /// # Errors
    ///
    /// Returns [`SchemeServerError`] when the price or chain is not supported.
    fn parse_price(&self, price: &str, network: &ChainId)
    -> Result<AssetAmount, SchemeServerError>;

    /// Fills in scheme-specific `extra` data on the requirements.
    ///
    /// Default implementation returns the input unchanged.
    fn enhance_requirements(&self, requirements: PaymentRequirements) -> PaymentRequirements {
        requirements
    }

    /// Builds a complete [`PaymentRequirements`] from a price.
    ///
    /// # Errors
    ///
    /// Returns [`SchemeServerError`] when the price cannot be resolved.
    fn build_requirements(
        &self,
        price: &str,
        network: &ChainId,
        pay_to: &str,
        max_timeout_seconds: u64,
    ) -> Result<PaymentRequirements, SchemeServerError> {
        let AssetAmount { asset, amount } = self.parse_price(price, network)?;
        let base = PaymentRequirements {
            scheme: CompactString::from(self.scheme()),
            network: network.clone(),
            amount,
            pay_to: CompactString::from(pay_to),
            max_timeout_seconds,
            asset,
            extra: None,
        };
        Ok(self.enhance_requirements(base))
    }
}
