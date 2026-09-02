//! Server-side price tag generation for the NEAR exact scheme.

use std::collections::HashMap;
use std::sync::LazyLock;

use r402_core::chain::{ChainId, DeployedTokenAmount};
use r402_core::wire;
use r402_core::{PaymentFlowConfig, SDK_DEFAULT_ASSET_TRANSFER_METHOD, SchemeNetworkServer};

use crate::chain::{NearAddress, NearTokenDeployment};
use crate::exact::{ExactScheme, NearExact};

fn near_exact_payment_flows() -> &'static HashMap<String, PaymentFlowConfig> {
    static FLOWS: LazyLock<HashMap<String, PaymentFlowConfig>> = LazyLock::new(|| {
        HashMap::from([(
            SDK_DEFAULT_ASSET_TRANSFER_METHOD.to_owned(),
            PaymentFlowConfig::authorization_and_upfront(),
        )])
    });
    &FLOWS
}

impl SchemeNetworkServer for NearExact {
    fn scheme(&self) -> &'static str {
        ExactScheme::VALUE
    }

    fn default_asset_transfer_method(&self) -> &'static str {
        SDK_DEFAULT_ASSET_TRANSFER_METHOD
    }

    fn payment_flows(&self) -> &HashMap<String, PaymentFlowConfig> {
        near_exact_payment_flows()
    }
}

impl NearExact {
    /// Creates a price tag for a NEP-141 token payment.
    ///
    /// The NEAR relayer is facilitator-local and is not surfaced on
    /// `PaymentRequirements.extra`.
    #[must_use]
    #[allow(
        clippy::needless_pass_by_value,
        reason = "DeployedTokenAmount is consumed for its fields"
    )]
    pub fn price_tag<A: Into<NearAddress>>(
        pay_to: A,
        asset: DeployedTokenAmount<u128, NearTokenDeployment>,
    ) -> wire::PriceTag {
        let chain_id: ChainId = asset.token.chain_reference.into();
        let requirements = wire::PaymentRequirements::new(
            ExactScheme.to_string().into(),
            chain_id,
            asset.amount.to_string().into(),
            pay_to.into().to_string().into(),
            asset.token.address.to_string().into(),
            300,
        );
        wire::PriceTag::new(requirements)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, reason = "test assertions")]
mod tests {
    use super::*;
    use crate::USDC;

    #[test]
    fn price_tag_omits_extra() {
        let pay_to: NearAddress = "merchant.testnet".parse().unwrap();
        let tag = NearExact::price_tag(pay_to, USDC::near_testnet().amount(1_000_000u128));
        assert_eq!(tag.requirements.scheme, "exact");
        assert_eq!(tag.requirements.network.to_string(), "near:testnet");
        assert!(tag.requirements.extra.is_none());
        assert_eq!(
            tag.requirements.asset,
            USDC::near_testnet().address.to_string()
        );
    }

    #[test]
    fn payment_flows_use_default_authorization_and_upfront() {
        let scheme = NearExact;
        assert_eq!(
            scheme
                .payment_flows()
                .get(SDK_DEFAULT_ASSET_TRANSFER_METHOD),
            Some(&PaymentFlowConfig::authorization_and_upfront())
        );
    }
}
