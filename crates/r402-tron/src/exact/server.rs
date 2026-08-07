//! Server-side price tag generation for the Tron exact scheme.
//!
//! This module provides functionality for servers to create price tags
//! that clients can use to generate payment authorizations.

use alloy_primitives::U256;
use r402_core::chain::{ChainId, DeployedTokenAmount};
use r402_core::wire;

use crate::chain::{Address, TronTokenDeployment};
use crate::exact::{AssetTransferMethod, ExactScheme, PaymentRequirementsExtra, TronExact};

impl TronExact {
    /// Creates a price tag for a TRC-20 token payment.
    ///
    /// `transfer_method` selects the on-chain mechanism:
    ///
    /// - `None` — defaults to `"eip3009"` per spec; requires `asset.token.tip712`
    ///   to be set (the token's TIP-712 domain name/version).
    /// - `Some(Permit2)` — SUN.io Permit2 via `x402ExactPermit2Proxy`;
    ///   `tip712` is optional (only used to populate informational `extra` fields).
    #[must_use]
    pub fn price_tag<A: Into<Address>>(
        pay_to: A,
        asset: &DeployedTokenAmount<U256, TronTokenDeployment>,
        transfer_method: Option<AssetTransferMethod>,
    ) -> wire::PriceTag {
        let chain_id: ChainId = asset.token.chain_reference.into();
        let extra = asset.token.tip712.as_ref().map(|tip712| {
            PaymentRequirementsExtra::new(
                tip712.name.clone(),
                tip712.version.clone(),
                transfer_method,
            )
        });
        let requirements = wire::PaymentRequirements::new(
            ExactScheme.to_string().into(),
            chain_id,
            asset.amount.to_string().into(),
            pay_to.into().to_string().into(),
            asset.token.address.to_string().into(),
            60,
        )
        .with_optional_extra(extra.and_then(|e| serde_json::to_value(e).ok()));
        wire::PriceTag {
            requirements,
            enricher: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chain::TronChainReference;

    #[test]
    fn price_tag_includes_tip712_domain() {
        let deployment = TronTokenDeployment::new(
            TronChainReference::MAINNET,
            Address::from_bytes([0u8; 20]),
            6,
        )
        .with_tip712("Tether USD", "1");
        let pay_to = Address::from_bytes([1u8; 20]);
        let tag = TronExact::price_tag(pay_to, &deployment.amount(U256::from(1_000_000u64)), None);
        assert_eq!(tag.requirements.scheme, "exact");
        assert_eq!(tag.requirements.network.to_string(), "tron:0x2b6653dc");
        let extra = tag.requirements.extra.unwrap();
        assert_eq!(extra["name"], "Tether USD");
        assert_eq!(extra["version"], "1");
    }
}
