//! Type definitions for the NEAR `"exact"` payment scheme.
//!
//! Wire format type aliases live in the [`v2`] sub-module.

pub use r402_protocol::scheme::ExactScheme;
use serde::{Deserialize, Serialize};

/// Extra fields for NEAR payment requirements.
///
/// NEAR exact payments carry no scheme-specific extra: the relayer is
/// facilitator-local. Empty objects on the wire deserialize; `None` is
/// omitted when serializing [`r402_protocol::payment::PaymentRequirements`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NearExtra {}

/// NEAR exact payment payload containing a Borsh `SignedDelegate`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExactNearPayload {
    /// Base64-encoded Borsh `SignedDelegate` (NEP-366).
    pub signed_delegate_action: String,
}

/// Wire format type aliases for the NEAR exact scheme.
///
/// Uses CAIP-2 chain IDs (`near:mainnet`, `near:testnet`) for chain
/// identification and embeds requirements directly in the payload.
pub mod v2 {
    use r402_protocol::payment;

    use super::{ExactNearPayload, ExactScheme, NearExtra};
    use crate::chain::{NearAddress, NearTokenAmount};

    /// Type alias for payment payloads with embedded requirements and NEAR-specific data.
    pub type PaymentPayload = payment::PaymentPayload<PaymentRequirements, ExactNearPayload>;

    /// Type alias for payment requirements with NEAR-specific types.
    pub type PaymentRequirements =
        payment::PaymentRequirements<ExactScheme, NearTokenAmount, NearAddress, NearExtra>;
}

#[cfg(test)]
#[allow(clippy::unwrap_used, reason = "test assertions")]
mod tests {
    use serde_json::Value;

    use super::*;

    #[test]
    fn payload_field_is_camel_case() {
        let payload = ExactNearPayload {
            signed_delegate_action: "AQID".to_owned(),
        };
        let json = serde_json::to_value(&payload).unwrap();
        assert_eq!(
            json.get("signedDelegateAction").and_then(Value::as_str),
            Some("AQID")
        );
        let back: ExactNearPayload = serde_json::from_value(json).unwrap();
        assert_eq!(back.signed_delegate_action, "AQID");
    }

    #[test]
    fn extra_rejects_unknown_fields() {
        let json = serde_json::json!({ "feePayer": "relayer.testnet" });
        assert!(serde_json::from_value::<NearExtra>(json).is_err());
        let empty = serde_json::json!({});
        assert!(serde_json::from_value::<NearExtra>(empty).is_ok());
    }
}
