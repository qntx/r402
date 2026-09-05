//! Server-side price tag generation for the Solana exact scheme.
//!
//! This module provides functionality for servers to create price tags
//! that clients can use to generate payment authorizations.

use std::collections::HashMap;
use std::future::Future;
use std::sync::LazyLock;

use r402_protocol::payment::{PaymentRequirements, PriceTag, SupportedResponse, V2};
use r402_protocol::{ChainId, DeployedTokenAmount};
use r402_server::{
    PaymentFlowConfig, SDK_DEFAULT_ASSET_TRANSFER_METHOD, SchemeNetworkServer,
    SchemePaymentRequiredContext,
};

use crate::chain::{Address, SolanaTokenDeployment};
use crate::exact::{ExactScheme, SolanaExact, SupportedPaymentKindExtra};

fn solana_exact_payment_flows() -> &'static HashMap<String, PaymentFlowConfig> {
    static FLOWS: LazyLock<HashMap<String, PaymentFlowConfig>> = LazyLock::new(|| {
        HashMap::from([(
            SDK_DEFAULT_ASSET_TRANSFER_METHOD.to_owned(),
            PaymentFlowConfig::authorization_and_upfront(),
        )])
    });
    &FLOWS
}

impl SchemeNetworkServer for SolanaExact {
    fn scheme(&self) -> &'static str {
        ExactScheme::VALUE
    }

    fn default_asset_transfer_method(&self) -> &'static str {
        SDK_DEFAULT_ASSET_TRANSFER_METHOD
    }

    fn payment_flows(&self) -> &HashMap<String, PaymentFlowConfig> {
        solana_exact_payment_flows()
    }

    fn enrich_payment_required_response<'a>(
        &'a self,
        ctx: &'a SchemePaymentRequiredContext<'a>,
    ) -> impl Future<Output = Option<Vec<PaymentRequirements>>> + Send + 'a {
        let mut accepts = ctx.requirements.to_vec();
        let changed = accepts.iter_mut().fold(false, |acc, req| {
            acc | (req.network == *ctx.network && apply_solana_fee_payer(req, ctx.supported))
        });
        std::future::ready(changed.then_some(accepts))
    }
}

impl SolanaExact {
    /// Creates a price tag for a Solana SPL token payment.
    #[allow(
        clippy::needless_pass_by_value,
        reason = "DeployedTokenAmount is consumed for its fields"
    )]
    pub fn price_tag<A: Into<Address>>(
        pay_to: A,
        asset: DeployedTokenAmount<u64, SolanaTokenDeployment>,
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

fn apply_solana_fee_payer(req: &mut PaymentRequirements, capabilities: &SupportedResponse) -> bool {
    if req.scheme.as_str() != ExactScheme::VALUE || req.extra.is_some() {
        return false;
    }
    let extra = matching_kind_extra(capabilities, ExactScheme::VALUE, &req.network.to_string())
        .and_then(|extra| serde_json::from_value::<SupportedPaymentKindExtra>(extra.clone()).ok());
    let Some(extra) = extra else {
        return false;
    };
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

    #[test]
    fn payment_flows_match_official_exact_svm() {
        let scheme = SolanaExact;
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
