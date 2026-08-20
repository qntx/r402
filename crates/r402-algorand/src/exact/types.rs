//! Type definitions for the Algorand `"exact"` payment scheme.
//!
//! Wire format type aliases live in the [`v2`] sub-module.

pub use r402_core::scheme::ExactScheme;
use serde::{Deserialize, Serialize};

use crate::chain::AlgorandAddress;

/// Extra fields for Algorand payment requirements.
///
/// `feePayer` is the facilitator address that will sign a 0-amount payment
/// covering the group fee. Omitted when the buyer pays their own fees.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AlgorandExtra {
    /// Facilitator address that sponsors fees.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fee_payer: Option<AlgorandAddress>,
}

/// Algorand exact payment payload containing an atomic group.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExactAvmPayload {
    /// Base64-encoded canonical-msgpack transactions (signed or unsigned).
    pub payment_group: Vec<String>,
    /// Zero-based index of the ASA transfer that pays the resource server.
    pub payment_index: u32,
}

/// Wire format type aliases for the Algorand exact scheme.
///
/// `payTo` is a 58-char address and `asset` is a decimal ASA id, so both
/// wire as `compact_str::CompactString`.
pub mod v2 {
    use compact_str::CompactString;
    use r402_core::wire;
    use r402_core::wire::U64String;

    use super::{AlgorandExtra, ExactAvmPayload, ExactScheme};

    /// Type alias for payment payloads with embedded requirements and AVM data.
    pub type PaymentPayload = wire::PaymentPayload<PaymentRequirements, ExactAvmPayload>;

    /// Type alias for payment requirements with Algorand-specific types.
    pub type PaymentRequirements =
        wire::PaymentRequirements<ExactScheme, U64String, CompactString, AlgorandExtra>;
}

#[cfg(test)]
#[allow(clippy::unwrap_used, reason = "test assertions")]
mod tests {
    use serde_json::Value;

    use super::*;

    #[test]
    fn payload_fields_are_camel_case() {
        let payload = ExactAvmPayload {
            payment_group: vec!["AQID".to_owned()],
            payment_index: 1,
        };
        let json = serde_json::to_value(&payload).unwrap();
        assert_eq!(
            json.get("paymentGroup")
                .and_then(Value::as_array)
                .and_then(|a| a.first())
                .and_then(Value::as_str),
            Some("AQID")
        );
        assert_eq!(json.get("paymentIndex").and_then(Value::as_u64), Some(1));
        let back: ExactAvmPayload = serde_json::from_value(json).unwrap();
        assert_eq!(back.payment_index, 1);
    }

    #[test]
    fn extra_rejects_unknown_fields() {
        let json = serde_json::json!({ "memo": "nope" });
        assert!(serde_json::from_value::<AlgorandExtra>(json).is_err());
        let empty = serde_json::json!({});
        assert!(serde_json::from_value::<AlgorandExtra>(empty).is_ok());
    }

    #[test]
    fn spec_fixture_round_trips() {
        let json = include_str!("fixtures/spec_payment_group.json");
        let payload: ExactAvmPayload = serde_json::from_str(json).unwrap();
        assert_eq!(payload.payment_index, 1);
        assert_eq!(payload.payment_group.len(), 2);
        let encoded = serde_json::to_value(&payload).unwrap();
        assert!(encoded.get("paymentGroup").is_some());
        assert!(encoded.get("paymentIndex").is_some());
    }
}
