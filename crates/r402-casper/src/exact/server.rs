//! Server-side price tag generation for the Casper exact scheme.
//!
//! A resource server uses this to turn a wCSPR amount into the
//! `accepts[]` entry advertised in a `402 Payment Required` response.

use std::sync::Arc;

use r402_core::chain::{ChainId, DeployedTokenAmount};
use r402_core::wire;

use crate::chain::{Address, CasperTokenDeployment};
use crate::exact::{CasperExact, CasperPaymentRequirementsExtra, ExactScheme};
use crate::motes::Motes;

/// Default `maxTimeoutSeconds` advertised for Casper payments.
///
/// Casper finalises in well under a minute; five minutes leaves ample room
/// for a buyer to sign and for the facilitator to land the settlement
/// transaction, and matches the value used by the other chain crates.
pub const DEFAULT_MAX_TIMEOUT_SECONDS: u64 = 300;

impl CasperExact {
    /// Creates a price tag for a Casper CEP-18 token payment.
    ///
    /// The `extra` block carries the token's EIP-712 domain (`name`,
    /// `version`, `symbol`, `decimals`) — without it the buyer cannot compute
    /// the digest the CEP-18 contract will verify.
    #[allow(
        clippy::needless_pass_by_value,
        reason = "DeployedTokenAmount is consumed for its fields"
    )]
    pub fn price_tag<A: Into<Address>>(
        pay_to: A,
        asset: DeployedTokenAmount<Motes, CasperTokenDeployment>,
    ) -> wire::PriceTag {
        let chain_id: ChainId = asset.token.chain_reference.into();
        let extra = CasperPaymentRequirementsExtra::new(asset.token.name, asset.token.version)
            .with_decimals(asset.token.decimals);
        let requirements = wire::PaymentRequirements::new(
            ExactScheme.to_string().into(),
            chain_id,
            asset.amount.to_string().into(),
            pay_to.into().to_string().into(),
            asset.token.address.to_string().into(),
            DEFAULT_MAX_TIMEOUT_SECONDS,
        )
        .with_optional_extra(serde_json::to_value(extra).ok());

        wire::PriceTag {
            requirements,
            enricher: Some(Arc::new(casper_fee_payer_enricher)),
        }
    }
}

/// Enricher for Casper price tags — copies the facilitator's advertised
/// `feePayer` into `requirements.extra` while preserving the EIP-712 domain
/// fields the seller already set.
pub fn casper_fee_payer_enricher(
    price_tag: &mut wire::PriceTag,
    capabilities: &wire::SupportedResponse,
) {
    let network = price_tag.requirements.network.to_string();
    let Some(fee_payer) = capabilities
        .kinds
        .iter()
        .find(|kind| {
            wire::V2 == kind.x402_version
                && kind.scheme.as_str() == ExactScheme.as_ref()
                && kind.network.as_str() == network
        })
        .and_then(|kind| kind.extra.as_ref())
        .and_then(|extra| extra.get("feePayer"))
        .cloned()
    else {
        return;
    };

    match price_tag.requirements.extra.as_mut() {
        Some(serde_json::Value::Object(existing)) => {
            let _ = existing.insert("feePayer".to_owned(), fee_payer);
        }
        _ => {
            price_tag.requirements.extra = Some(serde_json::json!({ "feePayer": fee_payer }));
        }
    }
}

#[cfg(test)]
mod tests {
    use compact_str::CompactString;

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

    fn capabilities(fee_payer: &str) -> wire::SupportedResponse {
        let mut supported = wire::SupportedResponse::default();
        supported.kinds.push(
            wire::SupportedPaymentKind::new(2, "exact", "casper:casper-test")
                .with_extra(serde_json::json!({ "feePayer": fee_payer })),
        );
        supported
    }

    #[test]
    fn enricher_merges_fee_payer_without_dropping_domain() {
        let mut tag = CasperExact::price_tag(payee(), WCSPR::casper_test().amount(1));
        casper_fee_payer_enricher(&mut tag, &capabilities(PAYEE));
        let extra = tag.requirements.extra.as_ref().unwrap();
        assert_eq!(extra["feePayer"], PAYEE);
        assert_eq!(extra["name"], "Wrapped CSPR");
    }

    #[test]
    fn enricher_ignores_other_networks() {
        let mut tag = CasperExact::price_tag(payee(), WCSPR::casper_test().amount(1));
        let mut supported = wire::SupportedResponse::default();
        supported.kinds.push(
            wire::SupportedPaymentKind::new(2, "exact", "casper:casper")
                .with_extra(serde_json::json!({ "feePayer": PAYEE })),
        );
        casper_fee_payer_enricher(&mut tag, &supported);
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
    fn enricher_populates_extra_when_absent() {
        let mut tag = wire::PriceTag {
            requirements: wire::PaymentRequirements::new(
                CompactString::from("exact"),
                CasperChainReference::CASPER_TEST.into(),
                CompactString::from("1"),
                CompactString::from(PAYEE),
                CompactString::from(WCSPR::casper_test().address.to_string()),
                DEFAULT_MAX_TIMEOUT_SECONDS,
            ),
            enricher: None,
        };
        casper_fee_payer_enricher(&mut tag, &capabilities(PAYEE));
        assert_eq!(tag.requirements.extra.as_ref().unwrap()["feePayer"], PAYEE);
    }
}
