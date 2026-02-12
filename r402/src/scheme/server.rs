//! Server-side scheme abstractions for x402 payment handling.
//!
//! This module provides the trait that resource servers use to convert
//! human-readable prices into token amounts and enrich payment requirements
//! with scheme-specific data. This mirrors the Go SDK's `SchemeNetworkServer`
//! interface.

use crate::chain::ChainId;
use crate::proto::v2;

/// A resolved token amount ready for use in payment requirements.
#[derive(Debug, Clone)]
pub struct AssetAmount {
    /// The token contract address.
    pub asset: String,
    /// The amount in the token's smallest unit (e.g., "10000" for 0.01 USDC).
    pub amount: String,
}

/// Trait for server-side scheme processing.
///
/// Implementations convert human-readable prices into protocol-level
/// payment requirements. This allows resource servers to specify prices
/// as `"0.01"` instead of manually constructing the full
/// [`v2::PaymentRequirements`] with raw token amounts.
///
/// # Relationship to Other Traits
///
/// - [`Facilitator`](crate::facilitator::Facilitator) — Facilitator-side: verify and settle payments
/// - [`super::SchemeClient`] — Client-side: generate payment candidates
/// - **`SchemeServer`** — Server-side: build payment requirements
pub trait SchemeServer: super::SchemeId + Send + Sync {
    /// Converts a human-readable price into a token amount for the given network.
    ///
    /// For example, converts `"0.01"` on `eip155:8453` into
    /// `AssetAmount { asset: "0x833589...", amount: "10000" }` for USDC (6 decimals).
    ///
    /// # Errors
    ///
    /// Returns an error if the price cannot be parsed or the network is not supported.
    fn parse_price(
        &self,
        price: &str,
        network: &ChainId,
    ) -> Result<AssetAmount, Box<dyn std::error::Error>>;

    /// Enriches base payment requirements with scheme-specific data.
    ///
    /// Called after [`parse_price`](Self::parse_price) to add any extra fields
    /// needed by the scheme (e.g., fee payer addresses, nonce parameters).
    ///
    /// The default implementation returns the requirements unchanged.
    fn enhance_requirements(
        &self,
        requirements: v2::PaymentRequirements,
    ) -> v2::PaymentRequirements {
        requirements
    }

    /// Builds complete [`v2::PaymentRequirements`] from a human-readable price.
    ///
    /// This is a convenience method that combines [`parse_price`](Self::parse_price)
    /// and [`enhance_requirements`](Self::enhance_requirements).
    ///
    /// # Errors
    ///
    /// Returns an error if the price cannot be parsed or the network is not supported.
    fn build_requirements(
        &self,
        price: &str,
        network: &ChainId,
        pay_to: &str,
        max_timeout_seconds: u64,
    ) -> Result<v2::PaymentRequirements, Box<dyn std::error::Error>> {
        let asset_amount = self.parse_price(price, network)?;
        let base = v2::PaymentRequirements {
            scheme: self.scheme().to_owned(),
            network: network.clone(),
            amount: asset_amount.amount,
            pay_to: pay_to.to_owned(),
            max_timeout_seconds,
            asset: asset_amount.asset,
            extra: None,
        };
        Ok(self.enhance_requirements(base))
    }
}
