//! Type definitions for the XRPL `"exact"` payment scheme.
//!
//! Wire format type aliases live in the [`v2`] sub-module.

pub use r402_core::scheme::ExactScheme;
use serde::{Deserialize, Serialize};

use crate::chain::XrplClassicAddress;

/// How an XRPL `Payment` is sequenced.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum XrplAssetTransferMethod {
    /// Consume the payer account's current `Sequence`.
    #[default]
    Sequence,
    /// Consume a pre-created ticket (`Sequence = 0` plus `TicketSequence`).
    TicketSequence,
}

impl XrplAssetTransferMethod {
    /// Returns the wire value.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Sequence => "sequence",
            Self::TicketSequence => "ticketSequence",
        }
    }
}

/// Extra fields for XRPL payment requirements.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct XrplExtra {
    /// Must be `false`: the payer pays the XRPL transaction fee.
    pub are_fees_sponsored: bool,
    /// Selects sequence vs ticket sequencing. Defaults to `sequence` when omitted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub asset_transfer_method: Option<XrplAssetTransferMethod>,
    /// Invoice id committed into the signed `InvoiceID` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub invoice_id: Option<compact_str::CompactString>,
    /// Destination tag required by the receiver.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub destination_tag: Option<u32>,
    /// Issued-currency issuer; required for IOU payments.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub issuer: Option<XrplClassicAddress>,
}

impl XrplExtra {
    /// Extra with `areFeesSponsored: false` and no optional fields.
    #[must_use]
    pub const fn unsponsored() -> Self {
        Self {
            are_fees_sponsored: false,
            asset_transfer_method: None,
            invoice_id: None,
            destination_tag: None,
            issuer: None,
        }
    }
}

/// XRPL exact payment payload containing a signed transaction blob.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExactXrplPayload {
    /// Hex-encoded signed XRPL transaction blob.
    pub signed_tx_blob: String,
}

/// Wire format type aliases for the XRPL exact scheme.
pub mod v2 {
    use r402_core::wire;

    use super::{ExactScheme, ExactXrplPayload, XrplExtra};
    use crate::chain::{XrplAddress, XrplTokenAmount};

    /// Type alias for payment payloads with embedded requirements and XRPL-specific data.
    pub type PaymentPayload = wire::PaymentPayload<PaymentRequirements, ExactXrplPayload>;

    /// Type alias for payment requirements with XRPL-specific types.
    pub type PaymentRequirements =
        wire::PaymentRequirements<ExactScheme, XrplTokenAmount, XrplAddress, XrplExtra>;
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::indexing_slicing,
    reason = "test assertions"
)]
mod tests {
    use serde_json::Value;

    use super::*;

    #[test]
    fn payload_field_is_camel_case() {
        let payload = ExactXrplPayload {
            signed_tx_blob: "ABCD".to_owned(),
        };
        let json = serde_json::to_value(&payload).unwrap();
        assert_eq!(
            json.get("signedTxBlob").and_then(Value::as_str),
            Some("ABCD")
        );
        let back: ExactXrplPayload = serde_json::from_value(json).unwrap();
        assert_eq!(back.signed_tx_blob, "ABCD");
    }

    #[test]
    fn extra_rejects_unknown_fields_and_requires_are_fees_sponsored() {
        let json = serde_json::json!({ "feePayer": "rHb9CJAWyB4rj91VRWn96DkukG4bwdtyTh" });
        assert!(serde_json::from_value::<XrplExtra>(json).is_err());
        let missing = serde_json::json!({});
        assert!(serde_json::from_value::<XrplExtra>(missing).is_err());
        let ok = serde_json::json!({ "areFeesSponsored": false });
        let extra: XrplExtra = serde_json::from_value(ok).unwrap();
        assert!(!extra.are_fees_sponsored);
    }

    #[test]
    fn golden_requirements_fixture_roundtrips() {
        let raw = include_str!("fixtures/ts_payment_requirements.json");
        let reqs: v2::PaymentRequirements = serde_json::from_str(raw).unwrap();
        assert_eq!(reqs.network.to_string(), "xrpl:1");
        assert_eq!(reqs.asset.as_str(), "XRP");
        let extra = reqs.extra.unwrap();
        assert!(!extra.are_fees_sponsored);
        assert_eq!(extra.invoice_id.as_deref(), Some("INV-2026-XRPL-001"));
    }

    #[test]
    fn extra_roundtrip_optional_fields() {
        let extra = XrplExtra {
            are_fees_sponsored: false,
            asset_transfer_method: Some(XrplAssetTransferMethod::TicketSequence),
            invoice_id: Some("INV-1".into()),
            destination_tag: Some(12345),
            issuer: None,
        };
        let json = serde_json::to_value(&extra).unwrap();
        assert_eq!(json["assetTransferMethod"], "ticketSequence");
        assert_eq!(json["destinationTag"], 12345);
        let back: XrplExtra = serde_json::from_value(json).unwrap();
        assert_eq!(back, extra);
    }
}
