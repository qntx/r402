//! Wire payload types for the Stellar `"exact"` payment scheme.
//!
//! Wire format type aliases live in the [`v2`] sub-module.

pub use r402_protocol::scheme::ExactScheme;
use serde::{Deserialize, Serialize};

/// Extra fields for Stellar payment requirements.
///
/// `areFeesSponsored` is required and must be `true`. A non-sponsored flow
/// is not implemented.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StellarExtra {
    /// Whether the facilitator sponsors fees. Must be `true`.
    pub are_fees_sponsored: bool,
}

impl StellarExtra {
    /// Extra advertised on `/supported` for Stellar networks.
    #[must_use]
    pub const fn sponsored() -> Self {
        Self {
            are_fees_sponsored: true,
        }
    }
}

impl Default for StellarExtra {
    fn default() -> Self {
        Self::sponsored()
    }
}

/// Stellar exact payment payload containing a base64 transaction XDR.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExactStellarPayload {
    /// Base64-encoded Stellar transaction envelope XDR.
    pub transaction: String,
}

/// Wire format type aliases for the Stellar exact scheme.
///
/// Uses CAIP-2 chain IDs (`stellar:pubnet`, `stellar:testnet`) for chain
/// identification and embeds requirements directly in the payload.
pub mod v2 {
    use r402_protocol::payment::{PaymentPayload as WirePayload, PaymentRequirements as WireReqs};

    use super::{ExactScheme, ExactStellarPayload, StellarExtra};
    use crate::chain::{StellarAddress, StellarTokenAmount};

    /// Type alias for payment payloads with embedded requirements and Stellar-specific data.
    pub type PaymentPayload = WirePayload<PaymentRequirements, ExactStellarPayload>;

    /// Type alias for payment requirements with Stellar-specific types.
    pub type PaymentRequirements =
        WireReqs<ExactScheme, StellarTokenAmount, StellarAddress, StellarExtra>;
}

#[cfg(test)]
mod tests {
    use serde_json::Value;

    use super::*;

    #[test]
    fn payload_field_is_camel_case() {
        let payload = ExactStellarPayload {
            transaction: "AAAAAg==".to_owned(),
        };
        let json = serde_json::to_value(&payload).unwrap();
        assert_eq!(
            json.get("transaction").and_then(Value::as_str),
            Some("AAAAAg==")
        );
        let back: ExactStellarPayload = serde_json::from_value(json).unwrap();
        assert_eq!(back.transaction, "AAAAAg==");
    }

    #[test]
    fn extra_rejects_unknown_fields() {
        let json = serde_json::json!({ "areFeesSponsored": true, "feePayer": "G..." });
        assert!(serde_json::from_value::<StellarExtra>(json).is_err());
        let false_extra = serde_json::json!({ "areFeesSponsored": false });
        let unsponsored: StellarExtra = serde_json::from_value(false_extra).unwrap();
        assert!(!unsponsored.are_fees_sponsored);
        let ok = serde_json::json!({ "areFeesSponsored": true });
        let sponsored: StellarExtra = serde_json::from_value(ok).unwrap();
        assert!(sponsored.are_fees_sponsored);
    }
}
