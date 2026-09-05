//! Server-side price tag generation for the Casper exact scheme.

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

use crate::chain::motes::Motes;
use crate::chain::{Address, CasperTokenDeployment};
use crate::exact::{CasperExact, CasperPaymentRequirementsExtra};

/// Default `maxTimeoutSeconds` advertised for Casper payments.
pub const DEFAULT_MAX_TIMEOUT_SECONDS: u64 = 300;

fn casper_exact_payment_flows() -> &'static HashMap<String, PaymentFlowConfig> {
    static FLOWS: LazyLock<HashMap<String, PaymentFlowConfig>> = LazyLock::new(|| {
        HashMap::from([(
            SDK_DEFAULT_ASSET_TRANSFER_METHOD.to_owned(),
            PaymentFlowConfig::authorization_only(),
        )])
    });
    &FLOWS
}

impl SchemeNetworkServer for CasperExact {
    fn scheme(&self) -> &'static str {
        ExactScheme::VALUE
    }

    fn default_asset_transfer_method(&self) -> &'static str {
        SDK_DEFAULT_ASSET_TRANSFER_METHOD
    }

    fn payment_flows(&self) -> &HashMap<String, PaymentFlowConfig> {
        casper_exact_payment_flows()
    }

    fn enrich_payment_required_response<'a>(
        &'a self,
        ctx: &'a SchemePaymentRequiredContext<'a>,
    ) -> impl Future<Output = Option<Vec<PaymentRequirements>>> + Send + 'a {
        let mut accepts = ctx.requirements.to_vec();
        let changed = accepts.iter_mut().fold(false, |acc, req| {
            acc | (req.network == *ctx.network && apply_casper_fee_payer(req, ctx.supported))
        });
        std::future::ready(changed.then_some(accepts))
    }
}

impl CasperExact {
    /// Creates a price tag for a Casper CEP-18 token payment.
    ///
    /// The `extra` block carries the token's EIP-712 domain (`name`,
    /// `version`, `decimals`) — without it the buyer cannot compute the
    /// digest the CEP-18 contract will verify.
    #[must_use]
    #[allow(
        clippy::needless_pass_by_value,
        reason = "DeployedTokenAmount is consumed for its fields"
    )]
    pub fn price_tag<A: Into<Address>>(
        pay_to: A,
        asset: DeployedTokenAmount<Motes, CasperTokenDeployment>,
    ) -> PriceTag {
        let chain_id: ChainId = asset.token.chain_reference.into();
        let extra = CasperPaymentRequirementsExtra::new(asset.token.name, asset.token.version)
            .with_decimals(asset.token.decimals);
        let requirements = PaymentRequirements::new(
            ExactScheme.to_string().into(),
            chain_id,
            asset.amount.to_string().into(),
            pay_to.into().to_string().into(),
            asset.token.address.to_string().into(),
            DEFAULT_MAX_TIMEOUT_SECONDS,
        )
        .with_optional_extra(serde_json::to_value(extra).ok());

        PriceTag::new(requirements)
    }
}

/// Copies the facilitator's advertised `feePayer` into `requirements.extra`
/// while preserving the EIP-712 domain fields the seller already set.
fn apply_casper_fee_payer(req: &mut PaymentRequirements, capabilities: &SupportedResponse) -> bool {
    if req.scheme.as_str() != ExactScheme::VALUE {
        return false;
    }
    let network = req.network.to_string();
    let Some(fee_payer) = capabilities
        .kinds
        .iter()
        .find(|kind| {
            V2 == kind.x402_version
                && kind.scheme.as_str() == ExactScheme.as_ref()
                && kind.network.as_str() == network
        })
        .and_then(|kind| kind.extra.as_ref())
        .and_then(|extra| extra.get("feePayer"))
        .cloned()
    else {
        return false;
    };

    match req.extra.as_mut() {
        Some(serde_json::Value::Object(existing)) => {
            if existing.get("feePayer") == Some(&fee_payer) {
                return false;
            }
            let _ = existing.insert("feePayer".to_owned(), fee_payer);
        }
        _ => {
            req.extra = Some(serde_json::json!({ "feePayer": fee_payer }));
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use compact_str::CompactString;
    use r402_protocol::payment::SupportedPaymentKind;

    use super::*;
    use crate::chain::CasperChainReference;
    use crate::networks::WCSPR;

    const PAYEE: &str = "00fedcba0987654321fedcba0987654321fedcba0987654321fedcba0987654321";

    fn payee() -> Address {
        PAYEE.parse().unwrap()
    }

    #[test]
    fn price_tag_carries_casper_wire_fields() {
        let tag = CasperExact::price_tag(payee(), WCSPR::casper_test().amount(1_500_000_000));
        assert_eq!(tag.requirements.scheme, "exact");
        assert_eq!(tag.requirements.network.to_string(), "casper:casper-test");
        assert_eq!(tag.requirements.amount, "1500000000");
        assert_eq!(tag.requirements.pay_to, PAYEE);
        assert_eq!(
            tag.requirements.asset,
            WCSPR::casper_test().address.to_string()
        );
        assert_eq!(
            tag.requirements.max_timeout_seconds,
            DEFAULT_MAX_TIMEOUT_SECONDS
        );
    }

    #[test]
    fn price_tag_declares_the_eip712_domain() {
        let tag = CasperExact::price_tag(payee(), WCSPR::casper_test().amount(1));
        let extra = tag.requirements.extra.as_ref().unwrap();
        assert_eq!(extra["name"], "Wrapped CSPR");
        assert_eq!(extra["version"], "1");
        assert_eq!(extra["decimals"], "9");
    }

    #[test]
    fn price_tag_amount_is_base_units_not_decimal() {
        let tag = CasperExact::price_tag(payee(), WCSPR::casper_test().amount(1_000_000_000));
        assert_eq!(
            tag.requirements.amount, "1000000000",
            "wire amounts must stay in motes"
        );
    }

    fn capabilities(fee_payer: &str) -> SupportedResponse {
        let mut supported = SupportedResponse::default();
        supported.kinds.push(
            SupportedPaymentKind::new(2, "exact", "casper:casper-test")
                .with_extra(serde_json::json!({ "feePayer": fee_payer })),
        );
        supported
    }

    #[test]
    fn enrich_merges_fee_payer_without_dropping_domain() {
        let mut tag = CasperExact::price_tag(payee(), WCSPR::casper_test().amount(1));
        assert!(apply_casper_fee_payer(
            &mut tag.requirements,
            &capabilities(PAYEE)
        ));
        let extra = tag.requirements.extra.as_ref().unwrap();
        assert_eq!(extra["feePayer"], PAYEE);
        assert_eq!(extra["name"], "Wrapped CSPR");
    }

    #[test]
    fn enrich_ignores_other_networks() {
        let mut tag = CasperExact::price_tag(payee(), WCSPR::casper_test().amount(1));
        let mut supported = SupportedResponse::default();
        supported.kinds.push(
            SupportedPaymentKind::new(2, "exact", "casper:casper")
                .with_extra(serde_json::json!({ "feePayer": PAYEE })),
        );
        assert!(!apply_casper_fee_payer(&mut tag.requirements, &supported));
        assert!(
            tag.requirements
                .extra
                .as_ref()
                .unwrap()
                .get("feePayer")
                .is_none(),
            "fee payer from a different network must not leak in"
        );
    }

    #[test]
    fn enrich_populates_extra_when_absent() {
        let mut req = PaymentRequirements::new(
            CompactString::from("exact"),
            CasperChainReference::CASPER_TEST.into(),
            CompactString::from("1"),
            CompactString::from(PAYEE),
            CompactString::from(WCSPR::casper_test().address.to_string()),
            DEFAULT_MAX_TIMEOUT_SECONDS,
        );
        assert!(apply_casper_fee_payer(&mut req, &capabilities(PAYEE)));
        assert_eq!(req.extra.as_ref().unwrap()["feePayer"], PAYEE);
    }

    #[test]
    fn payment_flows_are_authorization_only() {
        let scheme = CasperExact;
        assert_eq!(
            scheme
                .payment_flows()
                .get(SDK_DEFAULT_ASSET_TRANSFER_METHOD),
            Some(&PaymentFlowConfig::authorization_only())
        );
    }
}
