//! Server-side price tag generation for the Aptos exact scheme.

use std::sync::Arc;

use r402_core::chain::{ChainId, DeployedTokenAmount};
use r402_core::wire;

use crate::chain::{AptosAddress, AptosTokenDeployment};
use crate::exact::{AptosExact, AptosExtra, ExactScheme};

impl AptosExact {
    /// Creates a price tag for a fungible-asset payment.
    ///
    /// The enricher copies `feePayer` from the facilitator `/supported` kind.
    #[must_use]
    #[allow(
        clippy::needless_pass_by_value,
        reason = "DeployedTokenAmount is consumed for its fields"
    )]
    pub fn price_tag<A: Into<AptosAddress>>(
        pay_to: A,
        asset: DeployedTokenAmount<u64, AptosTokenDeployment>,
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
        wire::PriceTag {
            requirements,
            enricher: Some(Arc::new(aptos_fee_payer_enricher)),
        }
    }
}

/// Copies `feePayer` from `/supported` onto the price tag extra.
pub fn aptos_fee_payer_enricher(
    price_tag: &mut wire::PriceTag,
    capabilities: &wire::SupportedResponse,
) {
    if price_tag.requirements.extra.is_some() {
        return;
    }

    let extra = capabilities
        .kinds
        .iter()
        .find(|kind| {
            wire::V2 == kind.x402_version
                && kind.scheme.as_str() == ExactScheme.as_ref()
                && kind.network.as_str() == price_tag.requirements.network.to_string()
        })
        .and_then(|kind| kind.extra.as_ref())
        .and_then(|extra| serde_json::from_value::<AptosExtra>(extra.clone()).ok());

    if let Some(extra) = extra {
        price_tag.requirements.extra = serde_json::to_value(extra).ok();
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, reason = "test assertions")]
mod tests {
    use super::*;
    use crate::USDC;
    use crate::chain::AptosAddress;

    #[test]
    fn price_tag_sets_enricher() {
        let pay_to: AptosAddress =
            "0x0000000000000000000000000000000000000000000000000000000000000001"
                .parse()
                .unwrap();
        let tag = AptosExact::price_tag(pay_to, USDC::aptos_testnet().amount(1_000_000u64));
        assert_eq!(tag.requirements.scheme, "exact");
        assert_eq!(tag.requirements.network.to_string(), "aptos:2");
        assert!(tag.requirements.extra.is_none());
        assert!(tag.enricher.is_some());
        assert_eq!(
            tag.requirements.asset,
            USDC::aptos_testnet().address.to_string()
        );
    }
}
