//! Type definitions for the Keeta `"exact"` payment scheme.
//!
//! Wire format type aliases live in the [`v2`] sub-module.

pub use r402_core::scheme::ExactScheme;
use serde::{Deserialize, Serialize};

/// Extra fields for Keeta payment requirements.
///
/// Facilitator `/supported` does not advertise extra (`getExtra` is
/// undefined). Sellers may still set `external` so the client copies it
/// onto the `SEND` operation.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct KeetaExtra {
    /// Optional `SEND.external` reference the client must copy.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub external: Option<compact_str::CompactString>,
}

/// Keeta exact payment payload containing a signed DER block.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExactKeetaPayload {
    /// Base64-encoded ASN.1 DER signed block.
    pub block: String,
}

/// Wire format type aliases for the Keeta exact scheme.
///
/// Uses CAIP-2 chain IDs (`keeta:21378`, `keeta:1413829460`) for chain
/// identification and embeds requirements directly in the payload.
pub mod v2 {
    use r402_core::wire;

    use super::{ExactKeetaPayload, ExactScheme, KeetaExtra};
    use crate::chain::{KeetaAddress, KeetaTokenAmount};

    /// Type alias for payment payloads with embedded requirements and Keeta-specific data.
    pub type PaymentPayload = wire::PaymentPayload<PaymentRequirements, ExactKeetaPayload>;

    /// Type alias for payment requirements with Keeta-specific types.
    pub type PaymentRequirements =
        wire::PaymentRequirements<ExactScheme, KeetaTokenAmount, KeetaAddress, KeetaExtra>;
}

#[cfg(test)]
#[allow(clippy::unwrap_used, reason = "test assertions")]
mod tests {
    use serde_json::Value;

    use super::*;

    #[test]
    fn payload_field_is_camel_case() {
        let payload = ExactKeetaPayload {
            block: "AQID".to_owned(),
        };
        let json = serde_json::to_value(&payload).unwrap();
        assert_eq!(json.get("block").and_then(Value::as_str), Some("AQID"));
        let back: ExactKeetaPayload = serde_json::from_value(json).unwrap();
        assert_eq!(back.block, "AQID");
    }

    #[test]
    fn extra_rejects_unknown_fields() {
        let json = serde_json::json!({ "feePayer": "keeta_abc" });
        assert!(serde_json::from_value::<KeetaExtra>(json).is_err());
        let empty = serde_json::json!({});
        assert!(serde_json::from_value::<KeetaExtra>(empty).is_ok());
        let with_external = serde_json::json!({ "external": "ref-1" });
        let extra: KeetaExtra = serde_json::from_value(with_external).unwrap();
        assert_eq!(extra.external.as_deref(), Some("ref-1"));
    }

    #[test]
    fn spec_der_round_trips_through_block_try_from() {
        let encoded = include_str!("fixtures/ts_block.b64").trim();
        let bytes = base64::Engine::decode(&base64::engine::general_purpose::STANDARD, encoded)
            .expect("spec fixture is valid base64");
        let block = keetanetwork_block::Block::try_from(bytes.as_slice())
            .expect("spec DER must decode and verify via Block::try_from");
        assert_eq!(
            block.to_bytes(),
            bytes.as_slice(),
            "Block::to_bytes must round-trip the TS/spec DER"
        );
    }

    #[test]
    fn spec_payment_payload_round_trips() {
        let raw = include_str!("fixtures/ts_payment_payload.json");
        let payload: v2::PaymentPayload = serde_json::from_str(raw).expect("wire PaymentPayload");
        assert_eq!(payload.accepted.network.to_string(), "keeta:1413829460");
        assert_eq!(payload.accepted.scheme.to_string(), "exact");
        assert_eq!(payload.accepted.amount.as_str(), "1000000000");
        assert_eq!(
            payload.accepted.asset.as_str(),
            "keeta_anyiff4v34alvumupagmdyosydeq24lc4def5mrpmmyhx3j6vj2uucckeqn52"
        );
        assert_eq!(
            payload.accepted.pay_to.as_str(),
            "keeta_aabravistgwbrkpl4euafuiualiwcemgvv2hnu7ci66i76naj4vm6tmeahmzria"
        );
        assert_eq!(payload.accepted.max_timeout_seconds, 60);
        assert!(payload.accepted.extra.is_none());
        let expected_block = include_str!("fixtures/ts_block.b64").trim();
        assert_eq!(payload.payload.block, expected_block);
    }
}
