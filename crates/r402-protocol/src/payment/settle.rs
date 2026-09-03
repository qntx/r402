//! Facilitator `/settle` request and response.

use compact_str::CompactString;
use serde::{Deserialize, Serialize};

use super::verify::{TypedVerifyRequest, VerifyRequest};
use super::{Base64Bytes, Extensions};
use crate::error::{ErrorReason, FacilitatorError, VerificationError};
use crate::scheme::SchemeSlug;

/// Wire-level settle request. Same JSON as [`VerifyRequest`], distinct type.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SettleRequest(serde_json::Value);

impl SettleRequest {
    /// Consumes the request and returns the raw JSON.
    #[must_use]
    pub fn into_json(self) -> serde_json::Value {
        self.0
    }

    /// Inspects the request for scheme routing.
    #[must_use]
    pub fn scheme_slug(&self) -> Option<SchemeSlug> {
        super::scheme_slug_from_json(&self.0)
    }

    /// CAIP-2 network identifier from `paymentRequirements.network`.
    #[must_use]
    pub fn network(&self) -> &str {
        super::network_from_json(&self.0)
    }

    /// Overrides `paymentRequirements.amount` in-place (upto scheme).
    ///
    /// # Errors
    ///
    /// Returns [`VerificationError::InvalidFormat`] when `paymentRequirements`
    /// is missing.
    pub fn set_settlement_amount(&mut self, amount: &str) -> Result<(), VerificationError> {
        let req = self
            .0
            .get_mut("paymentRequirements")
            .and_then(serde_json::Value::as_object_mut)
            .ok_or_else(|| {
                VerificationError::InvalidFormat(
                    "settle request missing paymentRequirements object".into(),
                )
            })?;
        let _ = req.insert(
            "amount".to_owned(),
            serde_json::Value::String(amount.to_owned()),
        );
        Ok(())
    }
}

impl From<serde_json::Value> for SettleRequest {
    fn from(value: serde_json::Value) -> Self {
        Self(value)
    }
}

impl From<VerifyRequest> for SettleRequest {
    fn from(request: VerifyRequest) -> Self {
        Self(request.into_json())
    }
}

#[allow(
    clippy::multiple_inherent_impl,
    reason = "from_settle is owned by the settle request type"
)]
impl<const V: u8, TPayload, TRequirements> TypedVerifyRequest<V, TPayload, TRequirements>
where
    Self: serde::de::DeserializeOwned,
{
    /// Decodes a raw [`SettleRequest`] into this typed variant.
    ///
    /// # Errors
    ///
    /// Returns [`VerificationError::InvalidFormat`] when deserialisation fails.
    pub fn from_settle(request: SettleRequest) -> Result<Self, VerificationError> {
        serde_json::from_value(request.into_json())
            .map_err(|e| VerificationError::InvalidFormat(e.to_string()))
    }
}

/// Settlement outcome returned by a facilitator.
///
/// `extension_responses` is the `EXTENSION-RESPONSES` sidechannel. It is not
/// serialized into JSON and is never merged into `extensions`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(into = "SettleResponseWire", try_from = "SettleResponseWire")]
#[non_exhaustive]
pub enum SettleResponse {
    /// Settlement succeeded.
    Success {
        /// Payer address on the target chain, if the facilitator included it.
        payer: Option<CompactString>,
        /// On-chain transaction hash / signature.
        transaction: CompactString,
        /// CAIP-2 chain identifier the transaction landed on.
        network: CompactString,
        /// Actual amount settled, in the token's smallest unit.
        amount: Option<CompactString>,
        /// Facilitator-attached extension data (JSON body).
        extensions: Extensions,
        /// Facilitator `EXTENSION-RESPONSES` sidechannel. Not buyer wire.
        extension_responses: Extensions,
        /// Scheme-specific additional data.
        extra: Option<serde_json::Value>,
    },
    /// Settlement failed.
    Failure {
        /// Wire-level reason code.
        reason: ErrorReason,
        /// Optional human-readable message.
        message: Option<CompactString>,
        /// Optional payer address if identifiable.
        payer: Option<CompactString>,
        /// Broadcast transaction hash. Empty when no transaction was submitted.
        /// Spec §9: MUST be non-empty when `reason` is [`ErrorReason::SettlementPending`].
        transaction: CompactString,
        /// CAIP-2 chain identifier on which settlement was attempted.
        network: CompactString,
        /// Facilitator-attached extension data (JSON body).
        extensions: Extensions,
        /// Facilitator `EXTENSION-RESPONSES` sidechannel. Not buyer wire.
        extension_responses: Extensions,
        /// Scheme-specific additional data.
        extra: Option<serde_json::Value>,
    },
}

impl SettleResponse {
    /// Whether settlement succeeded.
    #[must_use]
    pub const fn is_success(&self) -> bool {
        matches!(self, Self::Success { .. })
    }

    /// Sidechannel map (empty when the header was absent).
    #[must_use]
    pub const fn extension_responses(&self) -> &Extensions {
        match self {
            Self::Success {
                extension_responses,
                ..
            }
            | Self::Failure {
                extension_responses,
                ..
            } => extension_responses,
        }
    }

    /// Replaces the sidechannel map.
    pub fn set_extension_responses(&mut self, responses: Extensions) {
        match self {
            Self::Success {
                extension_responses,
                ..
            }
            | Self::Failure {
                extension_responses,
                ..
            } => *extension_responses = responses,
        }
    }

    /// Encodes a successful settlement as base64 for `Payment-Response`.
    ///
    /// Returns `None` for `Failure`. Use [`Self::encode_base64_any`] when the
    /// failure body must go on the wire.
    #[must_use]
    pub fn encode_base64(&self) -> Option<Base64Bytes> {
        if !self.is_success() {
            return None;
        }
        let json = serde_json::to_vec(self).ok()?;
        Some(Base64Bytes::encode(json))
    }

    /// Encodes any [`SettleResponse`] as base64 for `Payment-Response`.
    #[must_use]
    pub fn encode_base64_any(&self) -> Option<Base64Bytes> {
        let json = serde_json::to_vec(self).ok()?;
        Some(Base64Bytes::encode(json))
    }

    /// Builds a `Failure` response from a [`FacilitatorError`].
    ///
    /// The caller supplies `transaction` (`""` when no broadcast hash is known).
    /// Returns `None` for [`FacilitatorError::Transport`]: that is HTTP 502,
    /// not a 402 body.
    #[must_use]
    pub fn from_facilitator_error(
        error: &FacilitatorError,
        network: impl Into<CompactString>,
        transaction: impl Into<CompactString>,
    ) -> Option<Self> {
        let problem = error.as_payment_problem()?;
        let reason = problem.reason();
        let transaction = transaction.into();
        debug_assert!(
            reason != ErrorReason::SettlementPending || !transaction.is_empty(),
            "settlement_pending requires a non-empty transaction hash"
        );
        Some(Self::Failure {
            reason,
            message: Some(CompactString::from(problem.details())),
            payer: None,
            transaction,
            network: network.into(),
            extensions: Extensions::new(),
            extension_responses: Extensions::new(),
            extra: None,
        })
    }

    /// `success: false`, `errorReason == settlement_pending`, and a non-empty hash.
    #[must_use]
    pub fn is_retryable_settlement_pending(&self) -> bool {
        match self {
            Self::Failure {
                reason: ErrorReason::SettlementPending,
                transaction,
                ..
            } => !transaction.is_empty(),
            _ => false,
        }
    }
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SettleResponseWire {
    success: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    error_reason: Option<ErrorReason>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    error_message: Option<CompactString>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    payer: Option<CompactString>,
    transaction: CompactString,
    network: CompactString,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    amount: Option<CompactString>,
    #[serde(default, skip_serializing_if = "Extensions::is_empty")]
    extensions: Extensions,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    extra: Option<serde_json::Value>,
}

impl From<SettleResponse> for SettleResponseWire {
    fn from(value: SettleResponse) -> Self {
        match value {
            SettleResponse::Success {
                payer,
                transaction,
                network,
                amount,
                extensions,
                extra,
                extension_responses: _,
            } => Self {
                success: true,
                error_reason: None,
                error_message: None,
                payer,
                transaction,
                network,
                amount,
                extensions,
                extra,
            },
            SettleResponse::Failure {
                reason,
                message,
                payer,
                transaction,
                network,
                extensions,
                extra,
                extension_responses: _,
            } => Self {
                success: false,
                error_reason: Some(reason),
                error_message: message,
                payer,
                transaction,
                network,
                amount: None,
                extensions,
                extra,
            },
        }
    }
}

impl TryFrom<SettleResponseWire> for SettleResponse {
    type Error = String;
    fn try_from(wire: SettleResponseWire) -> Result<Self, Self::Error> {
        if wire.success {
            Ok(Self::Success {
                payer: wire.payer,
                transaction: wire.transaction,
                network: wire.network,
                amount: wire.amount,
                extensions: wire.extensions,
                extension_responses: Extensions::new(),
                extra: wire.extra,
            })
        } else {
            Ok(Self::Failure {
                reason: wire.error_reason.ok_or("missing field: errorReason")?,
                message: wire.error_message,
                payer: wire.payer,
                transaction: wire.transaction,
                network: wire.network,
                extensions: wire.extensions,
                extension_responses: Extensions::new(),
                extra: wire.extra,
            })
        }
    }
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::panic,
    clippy::indexing_slicing,
    reason = "unit tests panic on assertion failure"
)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::error::FacilitatorTransportKind;
    use crate::payment::ExtensionEntry;

    fn v2_json(network: &str, scheme: &str) -> serde_json::Value {
        json!({
            "x402Version": 2,
            "paymentPayload": {
                "accepted": { "network": network, "scheme": scheme }
            },
            "paymentRequirements": { "network": network }
        })
    }

    #[test]
    fn settle_request_from_verify_preserves_slug() {
        let verify = VerifyRequest::from(v2_json("eip155:42161", "exact"));
        let settle: SettleRequest = verify.into();
        assert_eq!(
            settle.scheme_slug().unwrap().to_string(),
            "eip155:42161:exact"
        );
    }

    #[test]
    fn settle_request_network_missing_returns_empty() {
        let settle = SettleRequest::from(serde_json::json!({}));
        assert_eq!(settle.network(), "");
    }

    #[test]
    fn settle_amount_override_rewrites_payment_requirements() {
        let mut settle = SettleRequest::from(json!({
            "x402Version": 2,
            "paymentPayload": { "accepted": { "network": "eip155:8453", "scheme": "upto" } },
            "paymentRequirements": { "network": "eip155:8453", "amount": "5000000" }
        }));
        settle.set_settlement_amount("1500000").unwrap();
        let json = settle.into_json();
        assert_eq!(
            json["paymentRequirements"]["amount"].as_str(),
            Some("1500000")
        );
    }

    #[test]
    fn settle_amount_override_errors_when_requirements_missing() {
        let mut settle = SettleRequest::from(json!({}));
        let err = settle.set_settlement_amount("1").unwrap_err();
        assert!(matches!(err, VerificationError::InvalidFormat(_)));
    }

    #[test]
    fn settle_success_with_amount_roundtrip() {
        let response = SettleResponse::Success {
            payer: Some("0xABC".into()),
            transaction: "0xTX".into(),
            network: "eip155:8453".into(),
            amount: Some("1000000".into()),
            extensions: Extensions::new(),
            extension_responses: Extensions::new(),
            extra: None,
        };
        let encoded = serde_json::to_value(&response).unwrap();
        assert_eq!(encoded["success"], true);
        assert_eq!(encoded["amount"], "1000000");
        assert!(encoded.get("extensionResponses").is_none());
        let back: SettleResponse = serde_json::from_value(encoded).unwrap();
        assert_eq!(back, response);
    }

    #[test]
    fn settle_success_without_amount_is_valid() {
        let response = SettleResponse::Success {
            payer: Some("0xABC".into()),
            transaction: "0xTX".into(),
            network: "eip155:1".into(),
            amount: None,
            extensions: Extensions::new(),
            extension_responses: Extensions::new(),
            extra: None,
        };
        let encoded = serde_json::to_value(&response).unwrap();
        assert!(encoded.get("amount").is_none());
        let back: SettleResponse = serde_json::from_value(encoded).unwrap();
        assert_eq!(back, response);
    }

    #[test]
    fn settle_success_empty_transaction_allowed() {
        let json = json!({
            "success": true,
            "payer": "0xABC",
            "transaction": "",
            "network": "eip155:1"
        });
        let back: SettleResponse = serde_json::from_value(json).unwrap();
        assert!(matches!(
            back,
            SettleResponse::Success {
                ref transaction,
                payer: Some(ref payer),
                ..
            } if transaction.is_empty() && payer == "0xABC"
        ));
    }

    #[test]
    fn settle_success_empty_transaction_omitted_payer() {
        let json = json!({
            "success": true,
            "transaction": "",
            "network": "eip155:1"
        });
        let back: SettleResponse = serde_json::from_value(json).unwrap();
        assert!(matches!(
            back,
            SettleResponse::Success {
                ref transaction,
                payer: None,
                ..
            } if transaction.is_empty()
        ));
        let encoded = serde_json::to_value(&back).unwrap();
        assert_eq!(encoded["transaction"], "");
        assert!(encoded.get("payer").is_none());
    }

    #[test]
    fn settle_failure_roundtrip() {
        let response = SettleResponse::Failure {
            reason: ErrorReason::DuplicateSettlement,
            message: Some("already processed".into()),
            payer: None,
            transaction: CompactString::default(),
            network: "solana:mainnet".into(),
            extensions: Extensions::new(),
            extension_responses: Extensions::new(),
            extra: None,
        };
        let encoded = serde_json::to_value(&response).unwrap();
        assert_eq!(encoded["errorReason"], "duplicate_settlement");
        assert_eq!(encoded["transaction"], "");
        let back: SettleResponse = serde_json::from_value(encoded).unwrap();
        assert_eq!(back, response);
    }

    #[test]
    fn settle_failure_serializes_empty_transaction() {
        let response = SettleResponse::Failure {
            reason: ErrorReason::UnexpectedSettleError,
            message: None,
            payer: None,
            transaction: CompactString::default(),
            network: "eip155:8453".into(),
            extensions: Extensions::new(),
            extension_responses: Extensions::new(),
            extra: None,
        };
        let encoded = serde_json::to_value(&response).unwrap();
        assert_eq!(encoded["transaction"], "");
    }

    #[test]
    fn settle_failure_pending_roundtrip_preserves_transaction() {
        let response = SettleResponse::Failure {
            reason: ErrorReason::SettlementPending,
            message: Some("rpc timeout waiting for receipt".into()),
            payer: Some("0xpayer".into()),
            transaction: "0xabc".into(),
            network: "eip155:8453".into(),
            extensions: Extensions::new(),
            extension_responses: Extensions::new(),
            extra: None,
        };
        let encoded = serde_json::to_value(&response).unwrap();
        assert_eq!(encoded["success"], false);
        assert_eq!(encoded["errorReason"], "settlement_pending");
        assert_eq!(encoded["transaction"], "0xabc");
        assert!(encoded.get("extra").is_none());
        let back: SettleResponse = serde_json::from_value(encoded).unwrap();
        assert_eq!(back, response);
        assert!(back.is_retryable_settlement_pending());
    }

    #[test]
    fn settle_failure_pending_empty_transaction_is_not_retryable() {
        let response = SettleResponse::Failure {
            reason: ErrorReason::SettlementPending,
            message: None,
            payer: None,
            transaction: CompactString::default(),
            network: "eip155:8453".into(),
            extensions: Extensions::new(),
            extension_responses: Extensions::new(),
            extra: None,
        };
        assert!(!response.is_retryable_settlement_pending());
    }

    #[test]
    fn verify_and_settle_extra_roundtrip() {
        let extra = json!({"assetTransferMethod": "eip3009"});
        let settle = SettleResponse::Success {
            payer: Some("0xABC".into()),
            transaction: "0xTX".into(),
            network: "eip155:1".into(),
            amount: None,
            extensions: Extensions::new(),
            extension_responses: Extensions::new(),
            extra: Some(extra.clone()),
        };
        let settle_json = serde_json::to_value(&settle).unwrap();
        assert_eq!(settle_json["extra"]["assetTransferMethod"], "eip3009");
        let settle_back: SettleResponse = serde_json::from_value(settle_json).unwrap();
        assert_eq!(settle_back, settle);

        let failure = SettleResponse::Failure {
            reason: ErrorReason::UnexpectedSettleError,
            message: None,
            payer: None,
            transaction: CompactString::default(),
            network: "eip155:1".into(),
            extensions: Extensions::new(),
            extension_responses: Extensions::new(),
            extra: Some(extra),
        };
        let failure_json = serde_json::to_value(&failure).unwrap();
        assert_eq!(failure_json["extra"]["assetTransferMethod"], "eip3009");
        let failure_back: SettleResponse = serde_json::from_value(failure_json).unwrap();
        assert_eq!(failure_back, failure);
    }

    #[test]
    fn settle_deny_unknown_top_level_fields() {
        let settle_typo = json!({
            "success": true,
            "payer": "0xABC",
            "transaction": "0xTX",
            "network": "eip155:1",
            "extraTypo": {"k": 1}
        });
        assert!(serde_json::from_value::<SettleResponse>(settle_typo).is_err());
    }

    #[test]
    fn from_facilitator_error_carries_transaction() {
        let err = FacilitatorError::Onchain("rpc timeout".into());
        let response =
            SettleResponse::from_facilitator_error(&err, "eip155:8453", "0xpending").unwrap();
        match response {
            SettleResponse::Failure {
                transaction,
                network,
                extra,
                ..
            } => {
                assert_eq!(transaction, "0xpending");
                assert_eq!(network, "eip155:8453");
                assert!(extra.is_none());
            }
            SettleResponse::Success { .. } => panic!("expected failure"),
        }
    }

    #[test]
    fn from_facilitator_error_refuses_transport() {
        let err = FacilitatorError::transport(FacilitatorTransportKind::Timeout);
        assert!(SettleResponse::from_facilitator_error(&err, "eip155:8453", "").is_none());
    }

    #[test]
    fn settle_encode_base64_only_for_success() {
        let success = SettleResponse::Success {
            payer: Some("0xA".into()),
            transaction: "0xT".into(),
            network: "eip155:1".into(),
            amount: None,
            extensions: Extensions::new(),
            extension_responses: Extensions::new(),
            extra: None,
        };
        assert!(success.encode_base64().is_some());

        let failure = SettleResponse::Failure {
            reason: ErrorReason::UnexpectedSettleError,
            message: None,
            payer: None,
            transaction: CompactString::default(),
            network: "eip155:1".into(),
            extensions: Extensions::new(),
            extension_responses: Extensions::new(),
            extra: None,
        };
        assert!(failure.encode_base64().is_none());
        assert!(failure.encode_base64_any().is_some());
    }

    #[test]
    fn encode_base64_strips_extension_responses() {
        let mut success = SettleResponse::Success {
            payer: Some("0xA".into()),
            transaction: "0xT".into(),
            network: "eip155:1".into(),
            amount: None,
            extensions: Extensions::new(),
            extension_responses: Extensions::new(),
            extra: None,
        };
        let mut side = Extensions::new();
        side.insert("bazaar", ExtensionEntry::raw(json!({"status": "success"})));
        success.set_extension_responses(side);
        let bytes = success.encode_base64().unwrap();
        let json: serde_json::Value = serde_json::from_slice(&bytes.decode().unwrap()).unwrap();
        assert!(json.get("extensionResponses").is_none());
    }
}
