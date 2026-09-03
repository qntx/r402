//! Wire payload types for the Keeta `"exact"` payment scheme.
//!
//! Wire format type aliases live in the [`v2`] sub-module.

pub use r402_protocol::scheme::ExactScheme;
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
    use r402_protocol::payment::{
        PaymentPayload as WirePayload, PaymentRequirements as WireReqs, TypedVerifyRequest,
    };

    use super::{ExactKeetaPayload, ExactScheme, KeetaExtra};
    use crate::chain::{KeetaAddress, KeetaTokenAmount};

    /// Type alias for verify requests using the exact Keeta payment scheme.
    pub type VerifyRequest = TypedVerifyRequest<2, PaymentPayload, PaymentRequirements>;

    /// Type alias for settle requests (same structure as verify requests).
    pub type SettleRequest = VerifyRequest;

    /// Type alias for payment payloads with embedded requirements and Keeta-specific data.
    pub type PaymentPayload = WirePayload<PaymentRequirements, ExactKeetaPayload>;

    /// Type alias for payment requirements with Keeta-specific types.
    pub type PaymentRequirements =
        WireReqs<ExactScheme, KeetaTokenAmount, KeetaAddress, KeetaExtra>;
}

#[cfg(test)]
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
}
