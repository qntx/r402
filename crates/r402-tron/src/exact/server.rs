//! Server-side price tag generation for the Tron exact scheme.
//!
//! This module provides functionality for servers to create price tags
//! that clients can use to generate payment authorizations.

use std::collections::HashMap;
use std::sync::LazyLock;

use alloy_primitives::U256;
use r402_protocol::network::{ChainId, DeployedTokenAmount};
use r402_protocol::payment::{PaymentRequirements, PriceTag};
use r402_protocol::scheme::ExactScheme;
use r402_server::{PaymentFlowConfig, SchemeNetworkServer};

use crate::chain::{Address, TronTokenDeployment};
use crate::exact::{AssetTransferMethod, PaymentRequirementsExtra, TronExact};

fn tron_exact_payment_flows() -> &'static HashMap<String, PaymentFlowConfig> {
    static FLOWS: LazyLock<HashMap<String, PaymentFlowConfig>> = LazyLock::new(|| {
        let row = PaymentFlowConfig::authorization_and_upfront();
        HashMap::from([
            ("eip3009".to_owned(), row.clone()),
            ("permit2".to_owned(), row),
        ])
    });
    &FLOWS
}

impl SchemeNetworkServer for TronExact {
    fn scheme(&self) -> &'static str {
        ExactScheme::VALUE
    }

    fn default_asset_transfer_method(&self) -> &'static str {
        "eip3009"
    }

    fn payment_flows(&self) -> &HashMap<String, PaymentFlowConfig> {
        tron_exact_payment_flows()
    }
}

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
    ) -> PriceTag {
        let chain_id: ChainId = asset.token.chain_reference.into();
        let extra =
            PaymentRequirementsExtra::from_deployment(asset.token.tip712.clone(), transfer_method);
        let requirements = PaymentRequirements::new(
            ExactScheme.to_string().into(),
            chain_id,
            asset.amount.to_string().into(),
            pay_to.into().to_string().into(),
            asset.token.address.to_string().into(),
            300,
        )
        .with_optional_extra(extra);
        PriceTag::new(requirements)
    }
}

#[cfg(test)]
mod tests {
    use r402_server::PaymentFlowName;

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
        assert_eq!(extra.get("name"), Some(&serde_json::json!("Tether USD")));
        assert_eq!(extra.get("version"), Some(&serde_json::json!("1")));
    }

    #[test]
    fn price_tag_permit2_without_tip712_emits_method() {
        let deployment =
            TronTokenDeployment::new(TronChainReference::NILE, Address::from_bytes([0u8; 20]), 6);
        let pay_to = Address::from_bytes([1u8; 20]);
        let tag = TronExact::price_tag(
            pay_to,
            &deployment.amount(U256::from(1_000_000u64)),
            Some(AssetTransferMethod::Permit2),
        );
        let extra = tag.requirements.extra.expect("permit2 extra");
        assert_eq!(extra.get("name"), Some(&serde_json::json!("")));
        assert_eq!(extra.get("version"), Some(&serde_json::json!("")));
        assert_eq!(
            extra.get("assetTransferMethod"),
            Some(&serde_json::json!("permit2"))
        );
    }

    #[test]
    fn price_tag_without_tip712_or_method_omits_extra() {
        let deployment = TronTokenDeployment::new(
            TronChainReference::MAINNET,
            Address::from_bytes([0u8; 20]),
            6,
        );
        let pay_to = Address::from_bytes([1u8; 20]);
        let tag = TronExact::price_tag(pay_to, &deployment.amount(U256::from(1_000_000u64)), None);
        assert!(tag.requirements.extra.is_none());
    }

    #[test]
    fn payment_flows_match_eip3009_and_permit2() {
        let scheme = TronExact;
        assert_eq!(scheme.scheme(), "exact");
        assert_eq!(scheme.default_asset_transfer_method(), "eip3009");
        let flows = scheme.payment_flows();
        let expected = PaymentFlowConfig::authorization_and_upfront();
        assert_eq!(flows.get("eip3009"), Some(&expected));
        assert_eq!(flows.get("permit2"), Some(&expected));
        assert_eq!(expected.default, PaymentFlowName::Authorization);
    }
}
