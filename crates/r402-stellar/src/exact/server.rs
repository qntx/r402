//! Server-side price tag generation for the Stellar exact scheme.

use std::collections::HashMap;
use std::future::Future;
use std::sync::LazyLock;

use r402_protocol::network::{ChainId, DeployedTokenAmount};
use r402_protocol::payment::{PaymentRequirements, PriceTag, SupportedResponse, V2};
use r402_protocol::scheme::ExactScheme;
use r402_server::{
    PaymentFlowConfig, SDK_DEFAULT_ASSET_TRANSFER_METHOD, SchemeNetworkServer,
    SchemePaymentRequiredContext,
};

use crate::chain::{StellarAddress, StellarTokenDeployment};
use crate::exact::{StellarExact, StellarExtra};

fn stellar_exact_payment_flows() -> &'static HashMap<String, PaymentFlowConfig> {
    static FLOWS: LazyLock<HashMap<String, PaymentFlowConfig>> = LazyLock::new(|| {
        HashMap::from([(
            SDK_DEFAULT_ASSET_TRANSFER_METHOD.to_owned(),
            PaymentFlowConfig::authorization_and_upfront(),
        )])
    });
    &FLOWS
}

impl SchemeNetworkServer for StellarExact {
    fn scheme(&self) -> &'static str {
        ExactScheme::VALUE
    }

    fn default_asset_transfer_method(&self) -> &'static str {
        SDK_DEFAULT_ASSET_TRANSFER_METHOD
    }

    fn payment_flows(&self) -> &HashMap<String, PaymentFlowConfig> {
        stellar_exact_payment_flows()
    }

    fn enrich_payment_required_response<'a>(
        &'a self,
        ctx: &'a SchemePaymentRequiredContext<'a>,
    ) -> impl Future<Output = Option<Vec<PaymentRequirements>>> + Send + 'a {
        let mut accepts = ctx.requirements.to_vec();
        let changed = accepts.iter_mut().fold(false, |acc, req| {
            acc | (req.network == *ctx.network
                && apply_stellar_are_fees_sponsored(req, ctx.supported))
        });
        std::future::ready(changed.then_some(accepts))
    }
}

impl StellarExact {
    /// Creates a price tag for a SEP-41 token payment.
    ///
    /// Scheme enrich copies `areFeesSponsored: true` from `/supported`.
    #[must_use]
    #[allow(
        clippy::needless_pass_by_value,
        reason = "DeployedTokenAmount is consumed for its fields"
    )]
    pub fn price_tag<A: Into<StellarAddress>>(
        pay_to: A,
        asset: DeployedTokenAmount<i128, StellarTokenDeployment>,
    ) -> PriceTag {
        let chain_id: ChainId = asset.token.chain_reference.into();
        let requirements = PaymentRequirements::new(
            ExactScheme.to_string().into(),
            chain_id,
            asset.amount.to_string().into(),
            pay_to.into().to_string().into(),
            asset.token.address.to_string().into(),
            300,
        );
        PriceTag::new(requirements)
    }
}

fn apply_stellar_are_fees_sponsored(
    req: &mut PaymentRequirements,
    capabilities: &SupportedResponse,
) -> bool {
    if req.scheme.as_str() != ExactScheme::VALUE || req.extra.is_some() {
        return false;
    }
    let extra = matching_kind_extra(capabilities, ExactScheme::VALUE, &req.network.to_string())
        .and_then(|extra| serde_json::from_value::<StellarExtra>(extra.clone()).ok())
        .unwrap_or_else(StellarExtra::sponsored);
    let Ok(value) = serde_json::to_value(extra) else {
        return false;
    };
    req.extra = Some(value);
    true
}

fn matching_kind_extra<'a>(
    capabilities: &'a SupportedResponse,
    scheme: &str,
    network: &str,
) -> Option<&'a serde_json::Value> {
    capabilities.kinds.iter().find_map(|kind| {
        (V2 == kind.x402_version
            && kind.scheme.as_str() == scheme
            && kind.network.as_str() == network)
            .then_some(kind.extra.as_ref())
            .flatten()
    })
}

#[cfg(test)]
mod tests {
    use r402_server::PaymentFlowName;

    use super::*;
    use crate::USDC;
    use crate::chain::StellarAddress;

    #[test]
    fn price_tag_omits_extra_until_scheme_enrich() {
        let pay_to: StellarAddress = "GBBO4ZDDZTSM2IUKQYBAST3CFHNPFXECGEFTGWTA2WELR2BIWDK57UVE"
            .parse()
            .unwrap();
        let tag = StellarExact::price_tag(pay_to, USDC::stellar_testnet().amount(10_000_000));
        assert_eq!(tag.requirements.scheme, "exact");
        assert_eq!(tag.requirements.network.to_string(), "stellar:testnet");
        assert!(tag.requirements.extra.is_none());
        assert_eq!(
            tag.requirements.asset,
            USDC::stellar_testnet().address.to_string()
        );
    }

    #[test]
    fn enrich_sets_are_fees_sponsored_true() {
        let pay_to: StellarAddress = "GBBO4ZDDZTSM2IUKQYBAST3CFHNPFXECGEFTGWTA2WELR2BIWDK57UVE"
            .parse()
            .unwrap();
        let mut tag = StellarExact::price_tag(pay_to, USDC::stellar_testnet().amount(10_000_000));
        let supported = SupportedResponse::new().with_kinds(vec![
            r402_protocol::payment::SupportedPaymentKind::new(
                V2.into(),
                "exact",
                "stellar:testnet",
            )
            .with_extra(serde_json::json!({ "areFeesSponsored": true })),
        ]);
        assert!(apply_stellar_are_fees_sponsored(
            &mut tag.requirements,
            &supported
        ));
        let extra = tag.requirements.extra.unwrap();
        assert_eq!(extra["areFeesSponsored"], true);
    }

    #[test]
    fn payment_flows_use_default_authorization_and_upfront() {
        let scheme = StellarExact;
        assert_eq!(scheme.scheme(), "exact");
        assert_eq!(
            scheme.default_asset_transfer_method(),
            SDK_DEFAULT_ASSET_TRANSFER_METHOD
        );
        let row = scheme
            .payment_flows()
            .get(SDK_DEFAULT_ASSET_TRANSFER_METHOD)
            .expect("default ATM");
        assert_eq!(*row, PaymentFlowConfig::authorization_and_upfront());
        assert_eq!(row.default, PaymentFlowName::Authorization);
        assert!(scheme.dynamic_extra_fields().is_empty());
    }
}
