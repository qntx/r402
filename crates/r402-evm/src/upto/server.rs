//! Server-side price tag generation for the EIP-155 upto scheme.
//!
//! `paymentRequirements.extra.facilitatorAddress` is required for the buyer
//! to sign — the proxy enforces `msg.sender == witness.facilitator`.
//! [`SchemeNetworkServer::enrich_payment_required_response`] fills it from
//! `/supported` `signers` when the tag omitted it.

use std::collections::HashMap;
use std::future::Future;
use std::sync::LazyLock;

use alloy_primitives::{Address, U256};
use r402_protocol::network::{ChainId, DeployedTokenAmount};
use r402_protocol::payment::{PaymentRequirements, PriceTag, SupportedResponse};
use r402_server::{PaymentFlowConfig, SchemeNetworkServer, SchemePaymentRequiredContext};

use crate::chain::{ChecksummedAddress, Eip155TokenDeployment};
use crate::upto::{Eip155Upto, UptoAssetTransferMethod, UptoScheme};

fn eip155_upto_payment_flows() -> &'static HashMap<String, PaymentFlowConfig> {
    static FLOWS: LazyLock<HashMap<String, PaymentFlowConfig>> = LazyLock::new(|| {
        HashMap::from([(
            "permit2".to_owned(),
            PaymentFlowConfig::authorization_only(),
        )])
    });
    &FLOWS
}

impl SchemeNetworkServer for Eip155Upto {
    fn scheme(&self) -> &'static str {
        UptoScheme::VALUE
    }

    fn default_asset_transfer_method(&self) -> &'static str {
        "permit2"
    }

    fn payment_flows(&self) -> &HashMap<String, PaymentFlowConfig> {
        eip155_upto_payment_flows()
    }

    fn enrich_payment_required_response<'a>(
        &'a self,
        ctx: &'a SchemePaymentRequiredContext<'a>,
    ) -> impl Future<Output = Option<Vec<PaymentRequirements>>> + Send + 'a {
        let mut accepts = ctx.requirements.to_vec();
        let changed = accepts.iter_mut().fold(false, |acc, req| {
            acc | (req.scheme.as_str() == UptoScheme::VALUE
                && apply_facilitator_address(req, ctx.supported))
        });
        std::future::ready(changed.then_some(accepts))
    }
}

impl Eip155Upto {
    /// Creates a price tag for an EVM upto payment.
    ///
    /// `asset.amount` is the authorised maximum. Scheme enrich fills
    /// `extra.facilitatorAddress` from `/supported`. Use
    /// [`Self::price_tag_with_facilitator`] to pin it immediately.
    #[allow(
        clippy::needless_pass_by_value,
        reason = "mirrors Eip155Exact::price_tag signature for API parity"
    )]
    pub fn price_tag<A: Into<ChecksummedAddress>>(
        pay_to: A,
        asset: DeployedTokenAmount<U256, Eip155TokenDeployment>,
    ) -> PriceTag {
        let chain_id: ChainId = asset.token.chain_reference.into();
        let extra = serde_json::json!({
            "assetTransferMethod": UptoAssetTransferMethod::Permit2,
        });
        let requirements = base_requirements(pay_to, &asset, &chain_id, Some(extra));
        PriceTag::new(requirements)
    }

    /// Creates a price tag with an explicit `facilitatorAddress`.
    #[allow(
        clippy::needless_pass_by_value,
        reason = "mirrors Eip155Exact::price_tag signature for API parity"
    )]
    pub fn price_tag_with_facilitator<A: Into<ChecksummedAddress>, F: Into<ChecksummedAddress>>(
        pay_to: A,
        asset: DeployedTokenAmount<U256, Eip155TokenDeployment>,
        facilitator_address: F,
    ) -> PriceTag {
        let chain_id: ChainId = asset.token.chain_reference.into();
        let facilitator: ChecksummedAddress = facilitator_address.into();
        let extra = serde_json::json!({
            "assetTransferMethod": UptoAssetTransferMethod::Permit2,
            "facilitatorAddress": facilitator.to_string(),
        });
        let requirements = base_requirements(pay_to, &asset, &chain_id, Some(extra));
        PriceTag::new(requirements)
    }
}

fn base_requirements<A: Into<ChecksummedAddress>>(
    pay_to: A,
    asset: &DeployedTokenAmount<U256, Eip155TokenDeployment>,
    chain_id: &ChainId,
    extra: Option<serde_json::Value>,
) -> PaymentRequirements {
    PaymentRequirements::new(
        UptoScheme.to_string().into(),
        chain_id.clone(),
        asset.amount.to_string().into(),
        pay_to.into().to_string().into(),
        asset.token.address.to_string().into(),
        300,
    )
    .with_optional_extra(extra)
}

fn apply_facilitator_address(req: &mut PaymentRequirements, supported: &SupportedResponse) -> bool {
    if req
        .extra
        .as_ref()
        .and_then(serde_json::Value::as_object)
        .is_some_and(|extra| extra.contains_key("facilitatorAddress"))
    {
        return false;
    }
    let Some(facilitator) = pick_facilitator_address(supported, &req.network) else {
        return false;
    };
    let mut extra = req
        .extra
        .take()
        .and_then(|value| value.as_object().cloned())
        .unwrap_or_default();
    extra.insert(
        "facilitatorAddress".to_owned(),
        serde_json::Value::String(facilitator.to_checksum(None)),
    );
    req.extra = Some(serde_json::Value::Object(extra));
    true
}

fn pick_facilitator_address(supported: &SupportedResponse, chain_id: &ChainId) -> Option<Address> {
    supported
        .signers_for_chain(chain_id)
        .into_iter()
        .find_map(|s| s.parse::<Address>().ok())
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use alloy_primitives::{Address, U256};
    use compact_str::CompactString;

    use super::*;
    use crate::chain::{Eip155ChainReference, Eip155TokenDeployment};

    fn token_8453() -> Eip155TokenDeployment {
        Eip155TokenDeployment {
            chain_reference: Eip155ChainReference::new(8453),
            address: Address::ZERO,
            decimals: 6,
            eip712: None,
        }
    }

    fn supported_with_facilitator(facilitator: Address) -> SupportedResponse {
        let mut signers: HashMap<CompactString, Vec<CompactString>> = HashMap::new();
        let _ = signers.insert(
            "eip155:8453".into(),
            vec![facilitator.to_checksum(None).into()],
        );
        SupportedResponse::new().with_signers(signers)
    }

    #[test]
    fn payment_flows_match_official_upto_evm() {
        let scheme = Eip155Upto;
        assert_eq!(scheme.scheme(), "upto");
        assert_eq!(scheme.default_asset_transfer_method(), "permit2");
        assert_eq!(
            scheme.payment_flows().get("permit2"),
            Some(&PaymentFlowConfig::authorization_only())
        );
    }

    #[test]
    fn price_tag_starts_with_permit2_then_enrich_fills_facilitator() {
        let token = token_8453();
        let pay_to = ChecksummedAddress::from(Address::repeat_byte(0xCC));
        let mut tag = Eip155Upto::price_tag(pay_to, token.amount(U256::from(5_000_000_u64)));
        assert_eq!(tag.requirements.scheme.as_str(), "upto");
        assert_eq!(tag.requirements.network.to_string(), "eip155:8453");
        assert_eq!(tag.requirements.amount.as_str(), "5000000");
        assert_eq!(tag.requirements.max_timeout_seconds, 300);
        let extra = tag
            .requirements
            .extra
            .as_ref()
            .expect("permit2 ATM is advertised on the tag")
            .as_object()
            .expect("extra serialises to a JSON object");
        assert_eq!(
            extra
                .get("assetTransferMethod")
                .and_then(serde_json::Value::as_str),
            Some("permit2")
        );
        assert!(!extra.contains_key("facilitatorAddress"));

        let facilitator = Address::repeat_byte(0xFA);
        let supported = supported_with_facilitator(facilitator);
        assert!(apply_facilitator_address(&mut tag.requirements, &supported));

        let enriched = tag
            .requirements
            .extra
            .as_ref()
            .expect("scheme enrich should populate extra")
            .as_object()
            .expect("extra serialises to a JSON object");
        assert_eq!(
            enriched
                .get("assetTransferMethod")
                .and_then(serde_json::Value::as_str),
            Some("permit2")
        );
        assert_eq!(
            enriched
                .get("facilitatorAddress")
                .and_then(serde_json::Value::as_str),
            Some(facilitator.to_checksum(None).as_str())
        );
    }

    #[test]
    fn price_tag_with_facilitator_pins_address_immediately() {
        let token = token_8453();
        let pay_to = ChecksummedAddress::from(Address::repeat_byte(0xCC));
        let facilitator = Address::repeat_byte(0xFA);
        let tag = Eip155Upto::price_tag_with_facilitator(
            pay_to,
            token.amount(U256::from(1_000_000_u64)),
            facilitator,
        );
        let extra = tag
            .requirements
            .extra
            .as_ref()
            .expect("explicit facilitator should populate extra immediately")
            .as_object()
            .expect("extra serialises to a JSON object");
        assert_eq!(
            extra
                .get("assetTransferMethod")
                .and_then(serde_json::Value::as_str),
            Some("permit2")
        );
        assert_eq!(
            extra
                .get("facilitatorAddress")
                .and_then(serde_json::Value::as_str),
            Some(facilitator.to_checksum(None).as_str())
        );
    }

    #[test]
    fn enrich_is_idempotent_when_facilitator_already_set() {
        let token = token_8453();
        let pay_to = ChecksummedAddress::from(Address::repeat_byte(0xCC));
        let pinned = Address::repeat_byte(0xAA);
        let mut tag = Eip155Upto::price_tag_with_facilitator(
            pay_to,
            token.amount(U256::from(1_000_000_u64)),
            pinned,
        );
        let supported = supported_with_facilitator(Address::repeat_byte(0xFF));
        assert!(!apply_facilitator_address(
            &mut tag.requirements,
            &supported
        ));
        let extra = tag
            .requirements
            .extra
            .as_ref()
            .expect("extra preserved")
            .as_object()
            .expect("extra serialises to a JSON object");
        assert_eq!(
            extra
                .get("facilitatorAddress")
                .and_then(serde_json::Value::as_str),
            Some(pinned.to_checksum(None).as_str())
        );
    }
}
