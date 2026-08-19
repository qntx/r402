//! Server-side price tag generation for the XRPL exact scheme.

use std::sync::Arc;

use r402_core::chain::{ChainId, DeployedTokenAmount};
use r402_core::wire;

use crate::chain::{XrplAsset, XrplClassicAddress, XrplTokenAmount, XrplTokenDeployment};
use crate::exact::{ExactScheme, XrplExact, XrplExtra};

impl XrplExact {
    /// Creates a price tag for an XRPL payment.
    ///
    /// Sets `extra.areFeesSponsored = false`. IOU deployments also copy
    /// `extra.issuer`. The enricher copies `areFeesSponsored` (and
    /// `assetTransferMethod` when advertised) from `/supported`.
    #[must_use]
    #[allow(
        clippy::needless_pass_by_value,
        reason = "DeployedTokenAmount is consumed for its fields"
    )]
    pub fn price_tag<A: Into<XrplClassicAddress>>(
        pay_to: A,
        asset: DeployedTokenAmount<XrplTokenAmount, XrplTokenDeployment>,
    ) -> wire::PriceTag {
        let chain_id: ChainId = asset.token.chain_reference.into();
        let mut extra = XrplExtra::unsponsored();
        if let XrplAsset::Iou(iou) = &asset.token.asset {
            extra.issuer = Some(iou.issuer.clone());
        }
        let requirements = wire::PaymentRequirements::new(
            ExactScheme.to_string().into(),
            chain_id,
            asset.amount.as_str().into(),
            pay_to.into().to_string().into(),
            asset.token.asset.as_str().into(),
            300,
        )
        .with_optional_extra(serde_json::to_value(extra).ok());
        wire::PriceTag {
            requirements,
            enricher: Some(Arc::new(xrpl_fee_enricher_v2)),
        }
    }
}

/// Enricher: force `areFeesSponsored: false` and copy `assetTransferMethod`
/// from the matching `/supported` kind when the tag does not already set it.
pub fn xrpl_fee_enricher_v2(
    price_tag: &mut wire::PriceTag,
    capabilities: &wire::SupportedResponse,
) {
    let supported_extra = capabilities.kinds.iter().find(|kind| {
        wire::V2 == kind.x402_version
            && kind.scheme.as_str() == ExactScheme.as_ref()
            && kind.network.as_str() == price_tag.requirements.network.to_string()
    });
    let mut extra = price_tag
        .requirements
        .extra
        .clone()
        .unwrap_or_else(|| serde_json::json!({}));
    if let Some(obj) = extra.as_object_mut() {
        obj.insert("areFeesSponsored".to_owned(), serde_json::json!(false));
        if !obj.contains_key("assetTransferMethod")
            && let Some(atm) = supported_extra
                .and_then(|kind| kind.extra.as_ref())
                .and_then(|value| value.get("assetTransferMethod"))
        {
            obj.insert("assetTransferMethod".to_owned(), atm.clone());
        }
    }
    price_tag.requirements.extra = Some(extra);
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::indexing_slicing,
    reason = "test assertions"
)]
mod tests {
    use super::*;
    use crate::{RLUSD, XRP};

    #[test]
    fn price_tag_xrp_sets_unsponsored_extra() {
        let pay_to: XrplClassicAddress = "rHb9CJAWyB4rj91VRWn96DkukG4bwdtyTh".parse().unwrap();
        let tag = XrplExact::price_tag(pay_to, XRP::testnet().amount(1_000_000u64));
        assert_eq!(tag.requirements.scheme, "exact");
        assert_eq!(tag.requirements.network.to_string(), "xrpl:1");
        assert_eq!(tag.requirements.asset, "XRP");
        let extra = tag.requirements.extra.as_ref().unwrap();
        assert_eq!(extra["areFeesSponsored"], false);
        assert!(tag.enricher.is_some());
    }

    #[test]
    fn price_tag_rlusd_copies_issuer() {
        let pay_to: XrplClassicAddress = "rHb9CJAWyB4rj91VRWn96DkukG4bwdtyTh".parse().unwrap();
        let tag = XrplExact::price_tag(pay_to, RLUSD::testnet().amount("0.01"));
        let extra = tag.requirements.extra.as_ref().unwrap();
        assert_eq!(extra["issuer"], crate::RLUSD_TESTNET_ISSUER);
        assert_eq!(tag.requirements.asset, crate::RLUSD_CURRENCY);
    }

    #[test]
    fn enricher_sets_are_fees_sponsored_false() {
        let pay_to: XrplClassicAddress = "rHb9CJAWyB4rj91VRWn96DkukG4bwdtyTh".parse().unwrap();
        let mut tag = XrplExact::price_tag(pay_to, XRP::mainnet().amount(1u64));
        let capabilities = wire::SupportedResponse::new().with_kinds(vec![
            wire::SupportedPaymentKind::new(wire::V2.into(), "exact", "xrpl:0").with_extra(
                serde_json::json!({
                    "areFeesSponsored": false,
                    "assetTransferMethod": "ticketSequence"
                }),
            ),
        ]);
        xrpl_fee_enricher_v2(&mut tag, &capabilities);
        let extra = tag.requirements.extra.unwrap();
        assert_eq!(extra["areFeesSponsored"], false);
        assert_eq!(extra["assetTransferMethod"], "ticketSequence");
    }
}
