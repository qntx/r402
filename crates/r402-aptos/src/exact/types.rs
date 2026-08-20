//! Type definitions for the Aptos `"exact"` payment scheme.

pub use r402_core::scheme::ExactScheme;
use serde::{Deserialize, Serialize};

use crate::chain::AptosAddress;

/// Extra fields for Aptos payment requirements: optional facilitator `feePayer`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AptosExtra {
    /// Account that will pay gas and must sign at settle time.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fee_payer: Option<AptosAddress>,
}

/// Aptos exact payment payload: outer base64 of JSON-of-bytes.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExactAptosPayload {
    /// Base64(UTF-8 JSON `{ transaction: [u8], senderAuthenticator: [u8] }`).
    pub transaction: String,
}

/// Wire format type aliases for the Aptos exact scheme.
pub mod v2 {
    use r402_core::wire as proto_v2;

    use super::{AptosExtra, ExactAptosPayload, ExactScheme};
    use crate::chain::{AptosAddress, AptosTokenAmount};

    /// Type alias for verify requests using the exact Aptos payment scheme.
    pub type VerifyRequest = proto_v2::TypedVerifyRequest<2, PaymentPayload, PaymentRequirements>;

    /// Type alias for settle requests (same structure as verify requests).
    pub type SettleRequest = VerifyRequest;

    /// Type alias for payment payloads with embedded requirements and Aptos data.
    pub type PaymentPayload = proto_v2::PaymentPayload<PaymentRequirements, ExactAptosPayload>;

    /// Type alias for payment requirements with Aptos-specific types.
    pub type PaymentRequirements =
        proto_v2::PaymentRequirements<ExactScheme, AptosTokenAmount, AptosAddress, AptosExtra>;
}

#[cfg(test)]
#[allow(clippy::unwrap_used, reason = "test assertions")]
mod tests {
    use serde_json::Value;

    use super::*;

    #[test]
    fn payload_field_is_camel_case() {
        let payload = ExactAptosPayload {
            transaction: "AQID".to_owned(),
        };
        let json = serde_json::to_value(&payload).unwrap();
        assert_eq!(
            json.get("transaction").and_then(Value::as_str),
            Some("AQID")
        );
        let back: ExactAptosPayload = serde_json::from_value(json).unwrap();
        assert_eq!(back.transaction, "AQID");
    }

    #[test]
    fn extra_rejects_unknown_fields() {
        let json = serde_json::json!({ "feePayer": crate::chain::USDC_TESTNET_FA, "alias": "x" });
        assert!(serde_json::from_value::<AptosExtra>(json).is_err());
        let empty = serde_json::json!({});
        assert!(serde_json::from_value::<AptosExtra>(empty).is_ok());
    }

    #[test]
    fn extra_golden_fixture_roundtrips() {
        let raw = include_str!("fixtures/extra.json");
        let extra: AptosExtra = serde_json::from_str(raw).unwrap();
        assert_eq!(
            extra.fee_payer.as_ref().map(AptosAddress::as_str),
            Some("0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")
        );
        let encoded = serde_json::to_value(&extra).unwrap();
        assert_eq!(
            encoded,
            serde_json::json!({
                "feePayer": "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
            })
        );
    }
}
