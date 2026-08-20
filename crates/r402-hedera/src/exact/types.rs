//! Type definitions for the Hedera `"exact"` payment scheme.

pub use r402_core::scheme::ExactScheme;
use serde::{Deserialize, Serialize};

use crate::chain::HederaAddress;

/// Extra fields for Hedera payment requirements: facilitator `feePayer` only.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HederaExtra {
    /// Account that will pay network fees and must sign at settle time.
    pub fee_payer: HederaAddress,
}

/// Hedera exact payment payload containing a base64 protobuf transaction.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExactHederaPayload {
    /// Base64-encoded, payer-signed `TransferTransaction` (fee payer unset).
    pub transaction: String,
}

/// Wire format type aliases for the Hedera exact scheme.
pub mod v2 {
    use r402_core::wire as proto_v2;

    use super::{ExactHederaPayload, ExactScheme, HederaExtra};
    use crate::chain::{HederaAddress, HederaTokenAmount};

    /// Type alias for verify requests using the exact Hedera payment scheme.
    pub type VerifyRequest = proto_v2::TypedVerifyRequest<2, PaymentPayload, PaymentRequirements>;

    /// Type alias for settle requests (same structure as verify requests).
    pub type SettleRequest = VerifyRequest;

    /// Type alias for payment payloads with embedded requirements and Hedera-specific data.
    pub type PaymentPayload = proto_v2::PaymentPayload<PaymentRequirements, ExactHederaPayload>;

    /// Type alias for payment requirements with Hedera-specific types.
    pub type PaymentRequirements =
        proto_v2::PaymentRequirements<ExactScheme, HederaTokenAmount, HederaAddress, HederaExtra>;
}

#[cfg(test)]
#[allow(clippy::unwrap_used, reason = "test assertions")]
mod tests {
    use serde_json::Value;

    use super::*;

    #[test]
    fn payload_field_is_camel_case() {
        let payload = ExactHederaPayload {
            transaction: "AQID".to_owned(),
        };
        let json = serde_json::to_value(&payload).unwrap();
        assert_eq!(
            json.get("transaction").and_then(Value::as_str),
            Some("AQID")
        );
        let back: ExactHederaPayload = serde_json::from_value(json).unwrap();
        assert_eq!(back.transaction, "AQID");
    }

    #[test]
    fn extra_rejects_unknown_fields() {
        let json = serde_json::json!({ "feePayer": "0.0.5001", "aliasPolicy": "allow" });
        assert!(serde_json::from_value::<HederaExtra>(json).is_err());
        let ok = serde_json::json!({ "feePayer": "0.0.5001" });
        let extra: HederaExtra = serde_json::from_value(ok).unwrap();
        assert_eq!(extra.fee_payer.as_str(), "0.0.5001");
    }

    #[test]
    fn extra_golden_fixture_roundtrips() {
        let raw = include_str!("fixtures/extra.json");
        let extra: HederaExtra = serde_json::from_str(raw).unwrap();
        assert_eq!(extra.fee_payer.as_str(), "0.0.5001");
        let encoded = serde_json::to_value(&extra).unwrap();
        assert_eq!(encoded, serde_json::json!({ "feePayer": "0.0.5001" }));
    }
}
