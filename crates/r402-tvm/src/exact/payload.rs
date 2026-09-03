//! Type definitions for the TON `"exact"` payment scheme.

pub use r402_protocol::scheme::ExactScheme;
use serde::{Deserialize, Serialize};

use crate::chain::TvmAddress;
use crate::chain::codec::cell::{BocError, decode_base64_boc, make_zero_bit_cell};
use crate::exact::error::ERR_EXACT_TVM_INVALID_JETTON_TRANSFER;

/// Extra fields for TON payment requirements.
///
/// `areFeesSponsored` is required and must be `true` for this scheme.
/// Forward-payload fields are optional; omitted values compare against
/// TEP-74 defaults (`addr_none`, `"0"`, zero-bit payload).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TvmExtra {
    /// Whether the facilitator sponsors gas. Must be `true`.
    pub are_fees_sponsored: bool,
    /// Optional base64 BoC used as the jetton `forward_payload`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub forward_payload: Option<String>,
    /// Optional nanotons forwarded with the jetton transfer.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub forward_ton_amount: Option<String>,
    /// Optional `response_destination` for unused TON.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub response_destination: Option<TvmAddress>,
}

impl TvmExtra {
    /// Extra advertised on `/supported` for TVM networks.
    #[must_use]
    pub const fn sponsored() -> Self {
        Self {
            are_fees_sponsored: true,
            forward_payload: None,
            forward_ton_amount: None,
            response_destination: None,
        }
    }

    /// Effective `forward_ton_amount`, defaulting to `0`.
    ///
    /// # Errors
    ///
    /// Returns [`BocError`] if the string is not an unsigned integer.
    pub fn forward_ton_amount_u128(&self) -> Result<u128, BocError> {
        match self.forward_ton_amount.as_deref() {
            None => Ok(0),
            Some(s) => {
                if s.is_empty() || !s.bytes().all(|b| b.is_ascii_digit()) {
                    return Err(BocError::Invalid(
                        ERR_EXACT_TVM_INVALID_JETTON_TRANSFER.to_owned(),
                    ));
                }
                s.parse::<u128>().map_err(|_| {
                    BocError::Invalid(ERR_EXACT_TVM_INVALID_JETTON_TRANSFER.to_owned())
                })
            }
        }
    }

    /// Effective `response_destination` in raw form.
    #[must_use]
    pub fn effective_response_destination(&self) -> Option<&TvmAddress> {
        self.response_destination.as_ref()
    }

    /// Effective forward-payload cell.
    ///
    /// # Errors
    ///
    /// Returns [`BocError`] if `forward_payload` is set and is not a BoC.
    pub fn effective_forward_payload(&self) -> Result<tonlib_core::cell::Cell, BocError> {
        match self.forward_payload.as_deref() {
            Some(b64) => decode_base64_boc(b64),
            None => make_zero_bit_cell(),
        }
    }
}

impl Default for TvmExtra {
    fn default() -> Self {
        Self::sponsored()
    }
}

/// TON exact payment payload containing a settlement BoC and jetton minter.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExactTvmPayload {
    /// Base64-encoded TON internal message BoC.
    pub settlement_boc: String,
    /// Jetton master contract address (raw `workchain:hex`).
    pub asset: TvmAddress,
}

/// Wire format type aliases for the TON exact scheme.
pub mod v2 {
    use r402_protocol::payment;

    use super::{ExactScheme, ExactTvmPayload, TvmExtra};
    use crate::chain::{TvmAddress, TvmTokenAmount};

    /// Type alias for payment payloads with embedded requirements and TON-specific data.
    pub type PaymentPayload = payment::PaymentPayload<PaymentRequirements, ExactTvmPayload>;

    /// Type alias for payment requirements with TON-specific types.
    pub type PaymentRequirements =
        payment::PaymentRequirements<ExactScheme, TvmTokenAmount, TvmAddress, TvmExtra>;
}

#[cfg(test)]
#[allow(clippy::unwrap_used, reason = "test assertions")]
mod tests {
    use serde_json::Value;

    use super::*;

    #[test]
    fn payload_fields_are_camel_case() {
        let payload = ExactTvmPayload {
            settlement_boc: "te6cckEBAQEAAgAAAA==".to_owned(),
            asset: crate::USDT_TESTNET_MINTER.parse().unwrap(),
        };
        let json = serde_json::to_value(&payload).unwrap();
        assert_eq!(
            json.get("settlementBoc").and_then(Value::as_str),
            Some("te6cckEBAQEAAgAAAA==")
        );
        assert!(json.get("asset").is_some());
        let back: ExactTvmPayload = serde_json::from_value(json).unwrap();
        assert_eq!(back.settlement_boc, "te6cckEBAQEAAgAAAA==");
    }

    #[test]
    fn extra_rejects_unknown_fields() {
        let json = serde_json::json!({ "areFeesSponsored": true, "feePayer": "0:aa" });
        assert!(serde_json::from_value::<TvmExtra>(json).is_err());
        let ok = serde_json::json!({ "areFeesSponsored": true });
        let extra: TvmExtra = serde_json::from_value(ok).unwrap();
        assert!(extra.are_fees_sponsored);
        assert!(extra.forward_payload.is_none());
    }

    #[test]
    fn extra_golden_from_ts_shape() {
        let raw = include_str!("../../../../tests/fixtures/tvm/extra_sponsored.json");
        let extra: TvmExtra = serde_json::from_str(raw).unwrap();
        assert!(extra.are_fees_sponsored);
        assert!(extra.forward_payload.is_none());
        assert!(extra.forward_ton_amount.is_none());
        assert!(extra.response_destination.is_none());
    }
}
