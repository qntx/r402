//! Server-side price tag generation for the NEAR exact scheme.

use r402_core::chain::{ChainId, DeployedTokenAmount};
use r402_core::wire;

use crate::chain::{NearAddress, NearTokenDeployment};
use crate::exact::{ExactScheme, NearExact};

impl NearExact {
    /// Creates a price tag for a NEP-141 token payment.
    ///
    /// The enricher is `None`: the NEAR relayer is facilitator-local and is
    /// not surfaced on `PaymentRequirements.extra`.
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
        wire::PriceTag {
            requirements,
            enricher: None,
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, reason = "test assertions")]
mod tests {
    use super::*;
    use crate::USDC;
    use crate::chain::NearChainReference;

    #[test]
    fn price_tag_omits_extra_and_enricher() {
        let pay_to: NearAddress = "merchant.testnet".parse().unwrap();
        let tag = NearExact::price_tag(pay_to, USDC::near_testnet().amount(1_000_000u128));
        assert_eq!(tag.requirements.scheme, "exact");
        assert_eq!(tag.requirements.network.to_string(), "near:testnet");
        assert!(tag.requirements.extra.is_none());
        assert!(tag.enricher.is_none());
        assert_eq!(
            tag.requirements.asset,
            USDC::near_testnet().address.to_string()
        );
        let _ = NearChainReference::TESTNET;
    }
}
