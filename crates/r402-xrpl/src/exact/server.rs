//! Server-side price tag generation for the XRPL exact scheme.

use std::collections::HashMap;
use std::future::Future;
use std::sync::LazyLock;

use r402_core::chain::{ChainId, DeployedTokenAmount};
use r402_core::wire;
use r402_core::{PaymentFlowConfig, SchemeNetworkServer, SchemePaymentRequiredContext};

use crate::chain::{XrplAsset, XrplClassicAddress, XrplTokenAmount, XrplTokenDeployment};
use crate::exact::{ExactScheme, XrplExact, XrplExtra};

fn xrpl_exact_payment_flows() -> &'static HashMap<String, PaymentFlowConfig> {
    static FLOWS: LazyLock<HashMap<String, PaymentFlowConfig>> = LazyLock::new(|| {
        let row = PaymentFlowConfig::authorization_and_upfront();
        HashMap::from([
            ("sequence".to_owned(), row.clone()),
            ("ticketSequence".to_owned(), row),
        ])
    });
    &FLOWS
}

impl SchemeNetworkServer for XrplExact {
    fn scheme(&self) -> &'static str {
        ExactScheme::VALUE
    }

    fn default_asset_transfer_method(&self) -> &'static str {
        "sequence"
    }

    fn payment_flows(&self) -> &HashMap<String, PaymentFlowConfig> {
        xrpl_exact_payment_flows()
    }

    fn enrich_payment_required_response<'a>(
        &'a self,
        ctx: &'a SchemePaymentRequiredContext<'a>,
    ) -> impl Future<Output = Option<Vec<wire::PaymentRequirements>>> + Send + 'a {
        let mut accepts = ctx.requirements.to_vec();
        let changed = accepts
            .iter_mut()
            .fold(false, |acc, req| acc | apply_xrpl_unsponsored(req));
        std::future::ready(changed.then_some(accepts))
    }
}

impl XrplExact {
    /// Creates a price tag for an XRPL payment.
    ///
    /// Sets `extra.areFeesSponsored = false`. IOU deployments also copy
    /// `extra.issuer`. Scheme enrich forces `areFeesSponsored: false`.
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
        wire::PriceTag::new(requirements)
    }
}

/// Force `areFeesSponsored: false` without adding reserved ATM keys.
fn apply_xrpl_unsponsored(req: &mut wire::PaymentRequirements) -> bool {
    if req.scheme.as_str() != ExactScheme::VALUE {
        return false;
    }
    let mut extra = req.extra.clone().unwrap_or_else(|| serde_json::json!({}));
    let Some(obj) = extra.as_object_mut() else {
        return false;
    };
    let already_false = obj.get("areFeesSponsored") == Some(&serde_json::json!(false));
    obj.insert("areFeesSponsored".to_owned(), serde_json::json!(false));
    req.extra = Some(extra);
    !already_false
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
    fn enrich_keeps_are_fees_sponsored_false() {
        let pay_to: XrplClassicAddress = "rHb9CJAWyB4rj91VRWn96DkukG4bwdtyTh".parse().unwrap();
        let mut tag = XrplExact::price_tag(pay_to, XRP::mainnet().amount(1u64));
        assert!(!apply_xrpl_unsponsored(&mut tag.requirements));
        let extra = tag.requirements.extra.unwrap();
        assert_eq!(extra["areFeesSponsored"], false);
        assert!(extra.get("assetTransferMethod").is_none());
    }

    #[test]
    fn payment_flows_match_official_xrpl() {
        let scheme = XrplExact;
        assert_eq!(scheme.default_asset_transfer_method(), "sequence");
        let expected = PaymentFlowConfig::authorization_and_upfront();
        assert_eq!(scheme.payment_flows().get("sequence"), Some(&expected));
        assert_eq!(
            scheme.payment_flows().get("ticketSequence"),
            Some(&expected)
        );
    }
}
