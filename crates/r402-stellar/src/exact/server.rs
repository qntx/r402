//! Server-side price tag generation for the Stellar exact scheme.

use std::sync::Arc;

use r402_core::chain::{ChainId, DeployedTokenAmount};
use r402_core::wire;

use crate::chain::{StellarAddress, StellarTokenDeployment};
use crate::exact::{ExactScheme, StellarExact, StellarExtra};

impl StellarExact {
    /// Creates a price tag for a SEP-41 token payment.
    ///
    /// The enricher copies `areFeesSponsored: true` from `/supported`.
    #[must_use]
    #[allow(
        clippy::needless_pass_by_value,
        reason = "DeployedTokenAmount is consumed for its fields"
    )]
    pub fn price_tag<A: Into<StellarAddress>>(
        pay_to: A,
        asset: DeployedTokenAmount<i128, StellarTokenDeployment>,
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
            enricher: Some(Arc::new(stellar_are_fees_sponsored_enricher)),
        }
    }
}

/// Copies `areFeesSponsored: true` from the matching `/supported` kind.
pub fn stellar_are_fees_sponsored_enricher(
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
        .and_then(|extra| serde_json::from_value::<StellarExtra>(extra.clone()).ok())
        .unwrap_or_else(StellarExtra::sponsored);
    price_tag.requirements.extra = serde_json::to_value(extra).ok();
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::indexing_slicing,
    reason = "test assertions"
)]
mod tests {
    use super::*;
    use crate::USDC;

    #[test]
    fn price_tag_has_enricher() {
        let pay_to: StellarAddress = "GBBO4ZDDZTSM2IUKQYBAST3CFHNPFXECGEFTGWTA2WELR2BIWDK57UVE"
            .parse()
            .unwrap();
        let tag = StellarExact::price_tag(pay_to, USDC::stellar_testnet().amount(10_000_000));
        assert_eq!(tag.requirements.scheme, "exact");
        assert_eq!(tag.requirements.network.to_string(), "stellar:testnet");
        assert!(tag.requirements.extra.is_none());
        assert!(tag.enricher.is_some());
        assert_eq!(
            tag.requirements.asset,
            USDC::stellar_testnet().address.to_string()
        );
    }

    #[test]
    fn enricher_sets_are_fees_sponsored_true() {
        let pay_to: StellarAddress = "GBBO4ZDDZTSM2IUKQYBAST3CFHNPFXECGEFTGWTA2WELR2BIWDK57UVE"
            .parse()
            .unwrap();
        let mut tag = StellarExact::price_tag(pay_to, USDC::stellar_testnet().amount(10_000_000));
        let supported = wire::SupportedResponse::new().with_kinds(vec![
            wire::SupportedPaymentKind::new(wire::V2.into(), "exact", "stellar:testnet")
                .with_extra(serde_json::json!({ "areFeesSponsored": true })),
        ]);
        stellar_are_fees_sponsored_enricher(&mut tag, &supported);
        let extra = tag.requirements.extra.unwrap();
        assert_eq!(extra["areFeesSponsored"], true);
    }
}
