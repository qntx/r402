//! Server-side price tag generation for the EIP-155 exact scheme.
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

use crate::asset::AssetTransferMethod;
use crate::chain::{ChecksummedAddress, Eip155TokenDeployment};
use crate::exact::{Eip155Exact, PaymentRequirementsExtra};

fn eip155_exact_payment_flows() -> &'static HashMap<String, PaymentFlowConfig> {
    static FLOWS: LazyLock<HashMap<String, PaymentFlowConfig>> = LazyLock::new(|| {
        let row = PaymentFlowConfig::authorization_and_upfront();
        HashMap::from([
            ("eip3009".to_owned(), row.clone()),
            ("permit2".to_owned(), row),
        ])
    });
    &FLOWS
}

impl SchemeNetworkServer for Eip155Exact {
    fn scheme(&self) -> &'static str {
        ExactScheme::VALUE
    }

    fn default_asset_transfer_method(&self) -> &'static str {
        "eip3009"
    }

    fn payment_flows(&self) -> &HashMap<String, PaymentFlowConfig> {
        eip155_exact_payment_flows()
    }
}

impl Eip155Exact {
    /// Creates a price tag for an EVM exact payment.
    ///
    /// Generates a [`PriceTag`] that specifies the payment requirements for a
    /// resource. Uses CAIP-2 chain IDs (e.g., `eip155:8453`) and embeds the
    /// requirements directly in the price tag.
    ///
    /// # Transfer method
    ///
    /// - `None` or `Some(Eip3009)` — EIP-3009 `transferWithAuthorization` (default)
    /// - `Some(Permit2)` — Uniswap Permit2 via `x402Permit2Proxy`
    pub fn price_tag<A: Into<ChecksummedAddress>>(
        pay_to: A,
        asset: DeployedTokenAmount<U256, Eip155TokenDeployment>,
        transfer_method: Option<AssetTransferMethod>,
    ) -> PriceTag {
        let chain_id: ChainId = asset.token.chain_reference.into();
        let extra = PaymentRequirementsExtra::from_deployment(asset.token.eip712, transfer_method);
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

    #[test]
    fn payment_flows_match_official_exact_evm() {
        let scheme = Eip155Exact;
        assert_eq!(scheme.scheme(), "exact");
        assert_eq!(scheme.default_asset_transfer_method(), "eip3009");
        let flows = scheme.payment_flows();
        let expected = PaymentFlowConfig::authorization_and_upfront();
        assert_eq!(flows.get("eip3009"), Some(&expected));
        assert_eq!(flows.get("permit2"), Some(&expected));
        assert_eq!(expected.default, PaymentFlowName::Authorization);
    }
}
