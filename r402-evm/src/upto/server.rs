//! Server-side price tag generation for the EIP-155 upto scheme.
//!
//! Resource servers use [`Eip155Upto::price_tag`] to declare the **maximum**
//! amount clients may authorise. The actual amount charged at settlement is
//! determined by the server at request time via the
//! [`UptoActualAmount`](../../../../r402_http/server/upto/struct.UptoActualAmount.html)
//! HTTP response extension, and MUST be ≤ the authorised maximum.
//!
//! # Facilitator binding
//!
//! `paymentRequirements.extra.facilitatorAddress` is required for the buyer
//! to sign — the proxy contract enforces `msg.sender == witness.facilitator`
//! at settle time. The price tag installs a
//! [`wire::PriceTag::with_enricher`] callback that lifts the address from
//! the facilitator's `/supported` response (`signers` map indexed by
//! CAIP-2 chain). Resource servers can also pin an explicit address via
//! [`Eip155Upto::price_tag_with_facilitator`].

use std::sync::Arc;

use alloy_primitives::{Address, U256};
use r402_core::chain::{ChainId, DeployedTokenAmount};
use r402_core::wire;

use crate::chain::{ChecksummedAddress, Eip155TokenDeployment};
use crate::upto::{Eip155Upto, UptoAssetTransferMethod, UptoPaymentRequirementsExtra, UptoScheme};

impl Eip155Upto {
    /// Creates a price tag for an EVM upto payment.
    ///
    /// The `asset.amount` declares the authorised maximum; clients sign a
    /// Permit2 payload for exactly this value. The resource server MAY
    /// settle for any amount in `[0, max]` at request time.
    ///
    /// The returned tag carries an enricher that pulls
    /// `extra.facilitatorAddress` from the facilitator's `/supported`
    /// response (`signers` map). When the resource server already knows
    /// the facilitator address, prefer
    /// [`Self::price_tag_with_facilitator`] to avoid the extra round-trip.
    ///
    /// The default `maxTimeoutSeconds` is 300 s; override via
    /// [`wire::PriceTag::with_timeout`].
    #[allow(
        clippy::needless_pass_by_value,
        reason = "mirrors Eip155Exact::price_tag signature for API parity"
    )]
    pub fn price_tag<A: Into<ChecksummedAddress>>(
        pay_to: A,
        asset: DeployedTokenAmount<U256, Eip155TokenDeployment>,
    ) -> wire::PriceTag {
        let chain_id: ChainId = asset.token.chain_reference.into();
        let requirements = base_requirements(pay_to, asset, &chain_id, None);
        wire::PriceTag::new(requirements).with_enricher(Arc::new(
            move |tag: &mut wire::PriceTag, supported: &wire::SupportedResponse| {
                if tag.requirements.extra.is_some() {
                    return;
                }
                if let Some(facilitator) = pick_facilitator_address(supported, &chain_id) {
                    tag.requirements.extra = Some(serde_json::json!({
                        "assetTransferMethod": UptoAssetTransferMethod::Permit2,
                        "facilitatorAddress": facilitator.to_checksum(None),
                    }));
                }
            },
        ))
    }

    /// Creates a price tag with an explicit `facilitatorAddress`.
    ///
    /// Use this when the resource server already knows the facilitator's
    /// signer address and wants to avoid the `/supported` round-trip
    /// performed by the default [`Self::price_tag`] enricher.
    ///
    /// # Panics
    ///
    /// Never panics in practice: [`UptoPaymentRequirementsExtra`] is
    /// composed exclusively of values that round-trip through
    /// [`serde_json::Value`]. The `unwrap_or_default` guard keeps the API
    /// total even in the face of a future schema breakage.
    #[allow(
        clippy::needless_pass_by_value,
        reason = "consumes Address by value for ergonomic call sites"
    )]
    pub fn price_tag_with_facilitator<A: Into<ChecksummedAddress>, F: Into<ChecksummedAddress>>(
        pay_to: A,
        asset: DeployedTokenAmount<U256, Eip155TokenDeployment>,
        facilitator_address: F,
    ) -> wire::PriceTag {
        let chain_id: ChainId = asset.token.chain_reference.into();
        let extra = UptoPaymentRequirementsExtra::new(facilitator_address.into());
        let extra_json = serde_json::to_value(&extra).unwrap_or_default();
        let requirements = base_requirements(pay_to, asset, &chain_id, Some(extra_json));
        wire::PriceTag::new(requirements)
    }
}

#[allow(
    clippy::needless_pass_by_value,
    reason = "shape mirrors the public price_tag entry point so callers can\n             forward `asset` without re-borrowing"
)]
fn base_requirements<A: Into<ChecksummedAddress>>(
    pay_to: A,
    asset: DeployedTokenAmount<U256, Eip155TokenDeployment>,
    chain_id: &ChainId,
    extra: Option<serde_json::Value>,
) -> wire::PaymentRequirements {
    wire::PaymentRequirements {
        scheme: UptoScheme.to_string().into(),
        pay_to: pay_to.into().to_string().into(),
        asset: asset.token.address.to_string().into(),
        network: chain_id.clone(),
        amount: asset.amount.to_string().into(),
        max_timeout_seconds: 300,
        extra,
    }
}

fn pick_facilitator_address(
    supported: &wire::SupportedResponse,
    chain_id: &ChainId,
) -> Option<Address> {
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
    use r402_core::wire;

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

    #[test]
    fn price_tag_starts_without_extra_then_enricher_fills_facilitator() {
        let token = token_8453();
        let pay_to = ChecksummedAddress::from(Address::repeat_byte(0xCC));
        let mut tag = Eip155Upto::price_tag(pay_to, token.amount(U256::from(5_000_000_u64)));
        // Pre-enrich state: scheme and amount are set, extra is empty.
        assert_eq!(tag.requirements.scheme.as_str(), "upto");
        assert_eq!(tag.requirements.network.to_string(), "eip155:8453");
        assert_eq!(tag.requirements.amount.as_str(), "5000000");
        assert_eq!(tag.requirements.max_timeout_seconds, 300);
        assert!(tag.requirements.extra.is_none());

        // Simulate /supported response with one facilitator.
        let facilitator = Address::repeat_byte(0xFA);
        let mut signers: HashMap<CompactString, Vec<CompactString>> = HashMap::new();
        let _ = signers.insert(
            "eip155:8453".into(),
            vec![facilitator.to_checksum(None).into()],
        );
        let supported = wire::SupportedResponse {
            kinds: Vec::new(),
            extensions: Vec::new(),
            signers,
        };
        tag.enrich(&supported);

        let extra = tag
            .requirements
            .extra
            .as_ref()
            .expect("enricher should populate extra")
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
    fn enricher_is_idempotent_when_extra_already_set() {
        let token = token_8453();
        let pay_to = ChecksummedAddress::from(Address::repeat_byte(0xCC));
        let pinned = Address::repeat_byte(0xAA);
        let mut tag = Eip155Upto::price_tag_with_facilitator(
            pay_to,
            token.amount(U256::from(1_000_000_u64)),
            pinned,
        );
        // Run a default enricher equivalent: it must NOT overwrite the
        // already-pinned facilitatorAddress.
        let mut signers: HashMap<CompactString, Vec<CompactString>> = HashMap::new();
        let _ = signers.insert(
            "eip155:8453".into(),
            vec![Address::repeat_byte(0xFF).to_checksum(None).into()],
        );
        let supported = wire::SupportedResponse {
            kinds: Vec::new(),
            extensions: Vec::new(),
            signers,
        };
        tag.enrich(&supported);
        let extra = tag
            .requirements
            .extra
            .as_ref()
            .expect("extra preserved")
            .as_object()
            .expect("extra serialises to a JSON object");
        // Pinned address remains untouched.
        assert_eq!(
            extra
                .get("facilitatorAddress")
                .and_then(serde_json::Value::as_str),
            Some(pinned.to_checksum(None).as_str())
        );
    }
}
