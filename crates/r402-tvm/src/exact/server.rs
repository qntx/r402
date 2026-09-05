//! Server-side price tag generation for the TON exact scheme.

use std::collections::HashMap;
use std::future::Future;
use std::sync::LazyLock;

use r402_protocol::network::{ChainId, DeployedTokenAmount};
use r402_protocol::payment;
use r402_server::{
    PaymentFlowConfig, SDK_DEFAULT_ASSET_TRANSFER_METHOD, SchemeNetworkServer,
    SchemePaymentRequiredContext,
};

use crate::chain::{TvmAddress, TvmTokenDeployment};
use crate::exact::{ExactScheme, TvmExact, TvmExtra};

fn tvm_exact_payment_flows() -> &'static HashMap<String, PaymentFlowConfig> {
    static FLOWS: LazyLock<HashMap<String, PaymentFlowConfig>> = LazyLock::new(|| {
        HashMap::from([(
            SDK_DEFAULT_ASSET_TRANSFER_METHOD.to_owned(),
            PaymentFlowConfig::authorization_and_upfront(),
        )])
    });
    &FLOWS
}

impl SchemeNetworkServer for TvmExact {
    fn scheme(&self) -> &'static str {
        ExactScheme::VALUE
    }

    fn default_asset_transfer_method(&self) -> &'static str {
        SDK_DEFAULT_ASSET_TRANSFER_METHOD
    }

    fn payment_flows(&self) -> &HashMap<String, PaymentFlowConfig> {
        tvm_exact_payment_flows()
    }

    fn enrich_payment_required_response<'a>(
        &'a self,
        ctx: &'a SchemePaymentRequiredContext<'a>,
    ) -> impl Future<Output = Option<Vec<payment::PaymentRequirements>>> + Send + 'a {
        let mut accepts = ctx.requirements.to_vec();
        let changed = accepts.iter_mut().fold(false, |acc, req| {
            acc | (req.network == *ctx.network && apply_tvm_are_fees_sponsored(req, ctx.supported))
        });
        std::future::ready(changed.then_some(accepts))
    }
}

impl TvmExact {
    /// Creates a price tag for a jetton payment.
    ///
    /// Scheme enrich copies `areFeesSponsored: true` from `/supported`.
    #[must_use]
    #[allow(
        clippy::needless_pass_by_value,
        reason = "DeployedTokenAmount is consumed for its fields"
    )]
    pub fn price_tag<A: Into<TvmAddress>>(
        pay_to: A,
        asset: DeployedTokenAmount<u128, TvmTokenDeployment>,
    ) -> payment::PriceTag {
        let chain_id: ChainId = asset.token.chain_reference.into();
        let requirements = payment::PaymentRequirements::new(
            ExactScheme.to_string().into(),
            chain_id,
            asset.amount.to_string().into(),
            pay_to.into().to_string().into(),
            asset.token.address.to_string().into(),
            300,
        );
        payment::PriceTag::new(requirements)
    }
}

fn apply_tvm_are_fees_sponsored(
    req: &mut payment::PaymentRequirements,
    capabilities: &payment::SupportedResponse,
) -> bool {
    if req.scheme.as_str() != ExactScheme::VALUE || req.extra.is_some() {
        return false;
    }
    let extra = matching_kind_extra(capabilities, ExactScheme::VALUE, &req.network.to_string())
        .and_then(|extra| serde_json::from_value::<TvmExtra>(extra.clone()).ok())
        .unwrap_or_else(TvmExtra::sponsored);
    let Ok(value) = serde_json::to_value(extra) else {
        return false;
    };
    req.extra = Some(value);
    true
}

fn matching_kind_extra<'a>(
    capabilities: &'a payment::SupportedResponse,
    scheme: &str,
    network: &str,
) -> Option<&'a serde_json::Value> {
    capabilities.kinds.iter().find_map(|kind| {
        (payment::V2 == kind.x402_version
            && kind.scheme.as_str() == scheme
            && kind.network.as_str() == network)
            .then_some(kind.extra.as_ref())
            .flatten()
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, reason = "test assertions")]
mod tests {
    use super::*;
    use crate::USDT;

    #[test]
    fn price_tag_omits_extra_until_scheme_enrich() {
        let pay_to: TvmAddress = crate::USDT_TESTNET_MINTER.parse().unwrap();
        let tag = TvmExact::price_tag(pay_to, USDT::tvm_testnet().amount(10_000u128));
        assert_eq!(tag.requirements.scheme, "exact");
        assert_eq!(tag.requirements.network.to_string(), "tvm:-3");
        assert!(tag.requirements.extra.is_none());
    }

    #[test]
    fn payment_flows_use_default_authorization_and_upfront() {
        let scheme = TvmExact;
        assert_eq!(
            scheme
                .payment_flows()
                .get(SDK_DEFAULT_ASSET_TRANSFER_METHOD),
            Some(&PaymentFlowConfig::authorization_and_upfront())
        );
    }
}
