//! Server-side price tag generation for the TON exact scheme.

use std::sync::Arc;

use r402_core::chain::{ChainId, DeployedTokenAmount};
use r402_core::wire;

use crate::chain::{TvmAddress, TvmTokenDeployment};
use crate::exact::{ExactScheme, TvmExact, TvmExtra};

impl TvmExact {
    /// Creates a price tag for a jetton payment.
    ///
    /// The enricher copies `areFeesSponsored: true` from `/supported`.
    #[must_use]
    #[allow(
        clippy::needless_pass_by_value,
        reason = "DeployedTokenAmount is consumed for its fields"
    )]
    pub fn price_tag<A: Into<TvmAddress>>(
        pay_to: A,
        asset: DeployedTokenAmount<u128, TvmTokenDeployment>,
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
            enricher: Some(Arc::new(tvm_are_fees_sponsored_enricher)),
        }
    }
}

/// Copies `areFeesSponsored: true` from the matching `/supported` kind.
pub fn tvm_are_fees_sponsored_enricher(
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
        .and_then(|extra| serde_json::from_value::<TvmExtra>(extra.clone()).ok())
        .unwrap_or_else(TvmExtra::sponsored);
    price_tag.requirements.extra = serde_json::to_value(extra).ok();
}

#[cfg(test)]
#[allow(clippy::unwrap_used, reason = "test assertions")]
mod tests {
    use super::*;
    use crate::USDT;

    #[test]
    fn price_tag_has_enricher() {
        let pay_to: TvmAddress = crate::USDT_TESTNET_MINTER.parse().unwrap();
        let tag = TvmExact::price_tag(pay_to, USDT::tvm_testnet().amount(10_000u128));
        assert_eq!(tag.requirements.scheme, "exact");
        assert_eq!(tag.requirements.network.to_string(), "tvm:-3");
        assert!(tag.requirements.extra.is_none());
        assert!(tag.enricher.is_some());
    }
}
