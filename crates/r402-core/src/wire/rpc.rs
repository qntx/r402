//! Facilitator RPC envelopes: verify, settle, and `/supported`.

use std::collections::HashMap;
use std::str::FromStr;

use compact_str::CompactString;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use serde_with::{VecSkipError, serde_as};
use thiserror::Error;

use super::{Base64Bytes, Extensions, Version, Version2};
use crate::chain::ChainId;
use crate::error::{AsPaymentProblem, ErrorReason, FacilitatorError, VerificationError};
use crate::scheme::SchemeSlug;

/// A protocol-versioned verify request parameterized by payload and
/// requirements types.
///
/// The const parameter `V` selects the version marker. Client and
/// facilitator code that knows the concrete shape decodes a raw
/// [`VerifyRequest`] into [`TypedVerifyRequest`] via [`Self::from_verify`].
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TypedVerifyRequest<const V: u8, TPayload, TRequirements> {
    /// Protocol version marker.
    pub x402_version: Version<V>,
    /// The signed payment authorization.
    pub payment_payload: TPayload,
    /// The payment terms being verified.
    pub payment_requirements: TRequirements,
}

impl<const V: u8, TPayload, TRequirements> TypedVerifyRequest<V, TPayload, TRequirements>
where
    Self: serde::de::DeserializeOwned,
{
    /// Decodes a raw [`VerifyRequest`] into this typed variant.
    ///
    /// # Errors
    ///
    /// Returns [`VerificationError::InvalidFormat`] when deserialisation fails.
    pub fn from_verify(request: VerifyRequest) -> Result<Self, VerificationError> {
        serde_json::from_value(request.into_json())
            .map_err(|e| VerificationError::InvalidFormat(e.to_string()))
    }

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

impl<const V: u8, TPayload, TRequirements> TryFrom<TypedVerifyRequest<V, TPayload, TRequirements>>
    for VerifyRequest
where
    TPayload: Serialize,
    TRequirements: Serialize,
{
    type Error = serde_json::Error;
    fn try_from(
        value: TypedVerifyRequest<V, TPayload, TRequirements>,
    ) -> Result<Self, Self::Error> {
        let json = serde_json::to_value(value)?;
        Ok(Self(json))
    }
}

/// Wire-level verify request, stored as opaque JSON.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerifyRequest(serde_json::Value);

impl VerifyRequest {
    /// Consumes the request and returns the raw JSON.
    #[must_use]
    pub fn into_json(self) -> serde_json::Value {
        self.0
    }

    /// Inspects the request for scheme routing purposes without full decoding.
    #[must_use]
    pub fn scheme_slug(&self) -> Option<SchemeSlug> {
        scheme_slug_from_json(&self.0)
    }

    /// Returns the CAIP-2 network identifier from `paymentRequirements.network`.
    #[must_use]
    pub fn network(&self) -> &str {
        network_from_json(&self.0)
    }
}

impl From<serde_json::Value> for VerifyRequest {
    fn from(value: serde_json::Value) -> Self {
        Self(value)
    }
}

/// Wire-level settle request. Identical structure to [`VerifyRequest`] but
/// distinguished at the type level to prevent accidental misuse.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SettleRequest(serde_json::Value);

impl SettleRequest {
    /// Consumes the request and returns the raw JSON.
    #[must_use]
    pub fn into_json(self) -> serde_json::Value {
        self.0
    }

    /// Inspects the request for scheme routing purposes.
    #[must_use]
    pub fn scheme_slug(&self) -> Option<SchemeSlug> {
        scheme_slug_from_json(&self.0)
    }

    /// Returns the CAIP-2 network identifier from `paymentRequirements.network`.
    #[must_use]
    pub fn network(&self) -> &str {
        network_from_json(&self.0)
    }

    /// Overrides `paymentRequirements.amount` in-place.
    ///
    /// Intended for the **upto** scheme, where the resource server decides
    /// the actual settlement amount at request time (≤ the signed maximum).
    /// For the exact scheme this is a no-op: the amount must already equal
    /// what the buyer signed.
    ///
    /// # Errors
    ///
    /// Returns [`VerificationError::InvalidFormat`] when the JSON does not
    /// have a `paymentRequirements` object.
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

fn scheme_slug_from_json(json: &serde_json::Value) -> Option<SchemeSlug> {
    let version = json.get("x402Version")?.as_u64()?;
    let version: u8 = version.try_into().ok()?;
    if version != Version2::VALUE {
        return None;
    }
    let accepted = json.get("paymentPayload")?.get("accepted")?;
    let chain_id = ChainId::from_str(accepted.get("network")?.as_str()?).ok()?;
    let scheme = accepted.get("scheme")?.as_str()?;
    Some(SchemeSlug::new(chain_id, scheme.into()))
}

fn network_from_json(json: &serde_json::Value) -> &str {
    json.get("paymentRequirements")
        .and_then(|r| r.get("network"))
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default()
}

#[cfg(test)]
mod request_tests {
    use super::*;

    fn v2_json(network: &str, scheme: &str) -> serde_json::Value {
        serde_json::json!({
            "x402Version": 2,
            "paymentPayload": {
                "accepted": { "network": network, "scheme": scheme }
            },
            "paymentRequirements": { "network": network }
        })
    }

    #[test]
    fn verify_request_scheme_slug_evm() {
        let req = VerifyRequest::from(v2_json("eip155:8453", "exact"));
        let slug = req.scheme_slug().unwrap();
        assert_eq!(slug.to_string(), "eip155:8453:exact");
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
    fn slug_rejects_wrong_version() {
        let mut json = v2_json("eip155:1", "exact");
        json["x402Version"] = serde_json::json!(99);
        assert!(scheme_slug_from_json(&json).is_none());
    }

    #[test]
    fn slug_rejects_invalid_caip2() {
        assert!(scheme_slug_from_json(&v2_json("not-a-caip2", "exact")).is_none());
    }

    #[test]
    fn settle_amount_override_rewrites_payment_requirements() {
        let mut settle = SettleRequest::from(serde_json::json!({
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
        let mut settle = SettleRequest::from(serde_json::json!({}));
        let err = settle.set_settlement_amount("1").unwrap_err();
        assert!(matches!(err, VerificationError::InvalidFormat(_)));
    }
}

/// Verification outcome returned by a facilitator.
///
/// Serialised as a flat boolean-discriminated JSON object (matching the x402
/// wire format). Consumers get a safe, pattern-matchable Rust enum.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(into = "VerifyResponseWire", try_from = "VerifyResponseWire")]
#[non_exhaustive]
pub enum VerifyResponse {
    /// Payload passed all verification checks.
    Valid {
        /// Address of the verified payer.
        payer: CompactString,
        /// Facilitator-attached extension data.
        extensions: Extensions,
    },
    /// Payload was well-formed but failed verification.
    Invalid {
        /// Wire-level reason code.
        reason: ErrorReason,
        /// Optional human-readable message.
        message: Option<CompactString>,
        /// Optional payer address if identifiable from the payload.
        payer: Option<CompactString>,
        /// Facilitator-attached extension data.
        extensions: Extensions,
    },
}

impl VerifyResponse {
    /// Convenience: constructs a `Valid` response with no extensions.
    #[must_use]
    pub fn valid(payer: impl Into<CompactString>) -> Self {
        Self::Valid {
            payer: payer.into(),
            extensions: Extensions::new(),
        }
    }

    /// Convenience: constructs an `Invalid` response without a human message.
    #[must_use]
    pub fn invalid(payer: Option<CompactString>, reason: ErrorReason) -> Self {
        Self::Invalid {
            reason,
            message: None,
            payer,
            extensions: Extensions::new(),
        }
    }

    /// Convenience: constructs an `Invalid` response with a message.
    #[must_use]
    pub fn invalid_with_message(
        payer: Option<CompactString>,
        reason: ErrorReason,
        message: impl Into<CompactString>,
    ) -> Self {
        Self::Invalid {
            reason,
            message: Some(message.into()),
            payer,
            extensions: Extensions::new(),
        }
    }

    /// Returns `true` for `Valid` outcomes.
    #[must_use]
    pub const fn is_valid(&self) -> bool {
        matches!(self, Self::Valid { .. })
    }

    /// Converts a [`FacilitatorError`] into an `Invalid` response for the
    /// HTTP boundary, preserving the structured reason code.
    #[must_use]
    pub fn from_facilitator_error(error: &FacilitatorError) -> Self {
        let problem = error.as_payment_problem();
        Self::Invalid {
            reason: problem.reason(),
            message: Some(CompactString::from(problem.details())),
            payer: None,
            extensions: Extensions::new(),
        }
    }
}

/// Flat wire representation of [`VerifyResponse`].
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct VerifyResponseWire {
    is_valid: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    payer: Option<CompactString>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    invalid_reason: Option<ErrorReason>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    invalid_message: Option<CompactString>,
    #[serde(default, skip_serializing_if = "Extensions::is_empty")]
    extensions: Extensions,
}

impl From<VerifyResponse> for VerifyResponseWire {
    fn from(value: VerifyResponse) -> Self {
        match value {
            VerifyResponse::Valid { payer, extensions } => Self {
                is_valid: true,
                payer: Some(payer),
                invalid_reason: None,
                invalid_message: None,
                extensions,
            },
            VerifyResponse::Invalid {
                reason,
                message,
                payer,
                extensions,
            } => Self {
                is_valid: false,
                payer,
                invalid_reason: Some(reason),
                invalid_message: message,
                extensions,
            },
        }
    }
}

impl TryFrom<VerifyResponseWire> for VerifyResponse {
    type Error = String;
    fn try_from(wire: VerifyResponseWire) -> Result<Self, Self::Error> {
        if wire.is_valid {
            Ok(Self::Valid {
                payer: wire.payer.ok_or("missing field: payer")?,
                extensions: wire.extensions,
            })
        } else {
            Ok(Self::Invalid {
                reason: wire.invalid_reason.ok_or("missing field: invalidReason")?,
                message: wire.invalid_message,
                payer: wire.payer,
                extensions: wire.extensions,
            })
        }
    }
}

/// Settlement outcome returned by a facilitator.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(into = "SettleResponseWire", try_from = "SettleResponseWire")]
#[non_exhaustive]
pub enum SettleResponse {
    /// Settlement succeeded.
    Success {
        /// The payer address on the target chain.
        payer: CompactString,
        /// On-chain transaction hash / signature.
        transaction: CompactString,
        /// CAIP-2 chain identifier the transaction landed on.
        network: CompactString,
        /// Actual amount settled, in the token's smallest unit.
        ///
        /// Required when the `upto` scheme is in use; included for `exact` too
        /// to give clients unambiguous receipts.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        amount: Option<CompactString>,
        /// Facilitator-attached extension data.
        extensions: Extensions,
    },
    /// Settlement failed.
    Failure {
        /// Wire-level reason code.
        reason: ErrorReason,
        /// Optional human-readable message.
        message: Option<CompactString>,
        /// Optional payer address if identifiable.
        payer: Option<CompactString>,
        /// CAIP-2 chain identifier on which settlement was attempted.
        network: CompactString,
        /// Facilitator-attached extension data.
        extensions: Extensions,
    },
}

impl SettleResponse {
    /// Returns `true` when the settlement succeeded.
    #[must_use]
    pub const fn is_success(&self) -> bool {
        matches!(self, Self::Success { .. })
    }

    /// Encodes a successful settlement as base64 bytes for the
    /// `Payment-Response` HTTP header.
    ///
    /// Returns `None` for `Failure` variants to avoid accidentally
    /// transmitting a failed settlement as if it were successful. Call
    /// [`Self::encode_base64_any`] when the failure body is needed on the
    /// wire (for example, when a paygate must inform a browser client of a
    /// failed settlement via the `Payment-Response` header).
    #[must_use]
    pub fn encode_base64(&self) -> Option<Base64Bytes> {
        if !self.is_success() {
            return None;
        }
        let json = serde_json::to_vec(self).ok()?;
        Some(Base64Bytes::encode(json))
    }

    /// Encodes any [`SettleResponse`] (success or failure) as base64 bytes
    /// for the `Payment-Response` HTTP header.
    ///
    /// Use this when surfacing failed settlements is required by the spec,
    /// e.g. paygate error paths that still want to communicate the
    /// machine-readable error reason and chain to a browser client.
    /// Prefer [`Self::encode_base64`] when you only want to forward
    /// successful settlements.
    #[must_use]
    pub fn encode_base64_any(&self) -> Option<Base64Bytes> {
        let json = serde_json::to_vec(self).ok()?;
        Some(Base64Bytes::encode(json))
    }

    /// Builds a `Failure` response from a [`FacilitatorError`].
    #[must_use]
    pub fn from_facilitator_error(
        error: &FacilitatorError,
        network: impl Into<CompactString>,
    ) -> Self {
        let problem = error.as_payment_problem();
        Self::Failure {
            reason: problem.reason(),
            message: Some(CompactString::from(problem.details())),
            payer: None,
            network: network.into(),
            extensions: Extensions::new(),
        }
    }
}

/// Flat wire representation of [`SettleResponse`].
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
    /// Always serialized per x402 v2 §5.3.2: empty string on failure, and
    /// empty string on success for off-chain / deferred schemes (e.g.
    /// batch-settlement vouchers). On-chain schemes set a transaction hash.
    #[serde(default)]
    transaction: CompactString,
    network: CompactString,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    amount: Option<CompactString>,
    #[serde(default, skip_serializing_if = "Extensions::is_empty")]
    extensions: Extensions,
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
            } => Self {
                success: true,
                error_reason: None,
                error_message: None,
                payer: Some(payer),
                transaction,
                network,
                amount,
                extensions,
            },
            SettleResponse::Failure {
                reason,
                message,
                payer,
                network,
                extensions,
            } => Self {
                success: false,
                error_reason: Some(reason),
                error_message: message,
                payer,
                transaction: CompactString::default(),
                network,
                amount: None,
                extensions,
            },
        }
    }
}

impl TryFrom<SettleResponseWire> for SettleResponse {
    type Error = String;
    fn try_from(wire: SettleResponseWire) -> Result<Self, Self::Error> {
        if wire.success {
            let payer = wire.payer.ok_or("missing field: payer")?;
            Ok(Self::Success {
                payer,
                transaction: wire.transaction,
                network: wire.network,
                amount: wire.amount,
                extensions: wire.extensions,
            })
        } else {
            Ok(Self::Failure {
                reason: wire.error_reason.ok_or("missing field: errorReason")?,
                message: wire.error_message,
                payer: wire.payer,
                network: wire.network,
                extensions: wire.extensions,
            })
        }
    }
}

#[cfg(test)]
mod response_tests {
    use serde_json::json;

    use super::*;
    use crate::wire::ExtensionEntry;

    #[test]
    fn verify_valid_roundtrip() {
        let response = VerifyResponse::valid("0xABC");
        let encoded = serde_json::to_value(&response).unwrap();
        assert_eq!(encoded["isValid"], true);
        assert_eq!(encoded["payer"], "0xABC");
        assert!(encoded.get("invalidReason").is_none());

        let back: VerifyResponse = serde_json::from_value(encoded).unwrap();
        assert_eq!(back, response);
    }

    #[test]
    fn verify_valid_with_extensions_roundtrip() {
        let mut extensions = Extensions::new();
        extensions.insert(
            "payment-identifier",
            ExtensionEntry::info(json!({"id": "order-123"})),
        );
        let response = VerifyResponse::Valid {
            payer: "0xABC".into(),
            extensions,
        };
        let encoded = serde_json::to_value(&response).unwrap();
        assert_eq!(
            encoded["extensions"]["payment-identifier"]["info"]["id"],
            "order-123"
        );
        let back: VerifyResponse = serde_json::from_value(encoded).unwrap();
        assert_eq!(back, response);
    }

    #[test]
    fn verify_invalid_roundtrip() {
        let response = VerifyResponse::invalid_with_message(
            Some("0xDEF".into()),
            ErrorReason::InsufficientFunds,
            "not enough USDC",
        );
        let encoded = serde_json::to_value(&response).unwrap();
        assert_eq!(encoded["isValid"], false);
        assert_eq!(encoded["invalidReason"], "insufficient_funds");
        assert_eq!(encoded["invalidMessage"], "not enough USDC");
        let back: VerifyResponse = serde_json::from_value(encoded).unwrap();
        assert_eq!(back, response);
    }

    #[test]
    fn settle_success_with_amount_roundtrip() {
        let response = SettleResponse::Success {
            payer: "0xABC".into(),
            transaction: "0xTX".into(),
            network: "eip155:8453".into(),
            amount: Some("1000000".into()),
            extensions: Extensions::new(),
        };
        let encoded = serde_json::to_value(&response).unwrap();
        assert_eq!(encoded["success"], true);
        assert_eq!(encoded["amount"], "1000000");
        let back: SettleResponse = serde_json::from_value(encoded).unwrap();
        assert_eq!(back, response);
    }

    #[test]
    fn settle_success_without_amount_is_valid() {
        let response = SettleResponse::Success {
            payer: "0xABC".into(),
            transaction: "0xTX".into(),
            network: "eip155:1".into(),
            amount: None,
            extensions: Extensions::new(),
        };
        let encoded = serde_json::to_value(&response).unwrap();
        assert!(encoded.get("amount").is_none());
        let back: SettleResponse = serde_json::from_value(encoded).unwrap();
        assert_eq!(back, response);
    }

    #[test]
    fn settle_success_empty_transaction_allowed() {
        // Off-chain / deferred schemes (batch-settlement vouchers) use "".
        let json = json!({
            "success": true,
            "payer": "0xABC",
            "transaction": "",
            "network": "eip155:1"
        });
        let back: SettleResponse = serde_json::from_value(json).unwrap();
        assert!(matches!(
            back,
            SettleResponse::Success { ref transaction, .. } if transaction.is_empty()
        ));
    }

    #[test]
    fn settle_failure_roundtrip() {
        let response = SettleResponse::Failure {
            reason: ErrorReason::DuplicateSettlement,
            message: Some("already processed".into()),
            payer: None,
            network: "solana:mainnet".into(),
            extensions: Extensions::new(),
        };
        let encoded = serde_json::to_value(&response).unwrap();
        assert_eq!(encoded["errorReason"], "duplicate_settlement");
        let back: SettleResponse = serde_json::from_value(encoded).unwrap();
        assert_eq!(back, response);
    }

    /// Regression test for F-002: spec §5.3.2 marks `transaction` as Required
    /// (empty string on failure). Go SDK rejects responses lacking the field.
    #[test]
    fn settle_failure_serializes_empty_transaction() {
        let response = SettleResponse::Failure {
            reason: ErrorReason::UnexpectedSettleError,
            message: None,
            payer: None,
            network: "eip155:8453".into(),
            extensions: Extensions::new(),
        };
        let encoded = serde_json::to_value(&response).unwrap();
        assert_eq!(
            encoded["transaction"], "",
            "spec §5.3.2 requires `transaction` field present (empty on failure)"
        );
    }

    #[test]
    fn settle_encode_base64_only_for_success() {
        let success = SettleResponse::Success {
            payer: "0xA".into(),
            transaction: "0xT".into(),
            network: "eip155:1".into(),
            amount: None,
            extensions: Extensions::new(),
        };
        assert!(success.encode_base64().is_some());

        let failure = SettleResponse::Failure {
            reason: ErrorReason::UnexpectedSettleError,
            message: None,
            payer: None,
            network: "eip155:1".into(),
            extensions: Extensions::new(),
        };
        assert!(failure.encode_base64().is_none());
    }
}

/// A single payment kind advertised by a facilitator.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[non_exhaustive]
pub struct SupportedPaymentKind {
    /// x402 protocol version (`2`).
    pub x402_version: u8,
    /// Scheme name (e.g. `"exact"`, `"upto"`).
    pub scheme: CompactString,
    /// CAIP-2 network identifier.
    pub network: CompactString,
    /// Optional scheme-specific extras (fee payer, memo, ...).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extra: Option<serde_json::Value>,
}

impl SupportedPaymentKind {
    /// Constructs a kind from the three required fields. Use [`Self::with_extra`]
    /// to attach scheme-specific extras (fee payer, memo, etc.).
    #[must_use]
    pub fn new(
        x402_version: u8,
        scheme: impl Into<CompactString>,
        network: impl Into<CompactString>,
    ) -> Self {
        Self {
            x402_version,
            scheme: scheme.into(),
            network: network.into(),
            extra: None,
        }
    }

    /// Builder: attaches an `extra` JSON blob.
    #[must_use]
    pub fn with_extra(mut self, extra: serde_json::Value) -> Self {
        self.extra = Some(extra);
        self
    }

    /// Builder: attaches an optional `extra` blob, useful when the value is
    /// produced via `Option::map` upstream.
    #[must_use]
    pub fn with_optional_extra(mut self, extra: Option<serde_json::Value>) -> Self {
        self.extra = extra;
        self
    }
}

/// Response body of a facilitator's `/supported` endpoint.
///
/// Describes the full set of capabilities: payment kinds, known extensions,
/// and signer addresses keyed by CAIP-2 chain pattern.
#[serde_as]
#[derive(Clone, Default, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[non_exhaustive]
pub struct SupportedResponse {
    /// Supported payment kinds. Invalid entries are silently skipped.
    #[serde_as(as = "VecSkipError<_>")]
    pub kinds: Vec<SupportedPaymentKind>,
    /// Supported extension identifiers.
    #[serde(default)]
    pub extensions: Vec<CompactString>,
    /// Signer addresses indexed by CAIP-2 pattern
    /// (`"eip155:8453"`, `"solana:*"`, ...).
    #[serde(default)]
    pub signers: HashMap<CompactString, Vec<CompactString>>,
}

impl SupportedResponse {
    /// Constructs an empty response. Equivalent to [`Default::default`] but
    /// recommended for explicit construction sites where the field set will
    /// grow over time.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Builder: replaces the `kinds` list.
    #[must_use]
    pub fn with_kinds(mut self, kinds: Vec<SupportedPaymentKind>) -> Self {
        self.kinds = kinds;
        self
    }

    /// Builder: replaces the `extensions` identifier list.
    #[must_use]
    pub fn with_extensions(mut self, extensions: Vec<CompactString>) -> Self {
        self.extensions = extensions;
        self
    }

    /// Builder: replaces the per-pattern signer map.
    #[must_use]
    pub fn with_signers(mut self, signers: HashMap<CompactString, Vec<CompactString>>) -> Self {
        self.signers = signers;
        self
    }

    /// Returns all signer addresses that match the given chain.
    ///
    /// Matches both the exact pattern (`"eip155:8453"`) and the namespace
    /// wildcard (`"eip155:*"`).
    #[must_use]
    pub fn signers_for_chain(&self, chain_id: &ChainId) -> Vec<&str> {
        let exact = CompactString::from(chain_id.to_string());
        let wildcard = CompactString::from(format!("{}:*", chain_id.namespace()));
        let mut out = Vec::new();
        if let Some(list) = self.signers.get(&exact) {
            out.extend(list.iter().map(CompactString::as_str));
        }
        if let Some(list) = self.signers.get(&wildcard) {
            out.extend(list.iter().map(CompactString::as_str));
        }
        out
    }
}

/// Official wire shape for partial settlement (upto billing by actual usage).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub struct SettlementOverrides {
    /// Amount to settle. Formats: atomic `"1000"`, percent `"50%"`, dollar `"$0.05"`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub amount: Option<String>,
}

impl SettlementOverrides {
    /// Builds overrides with a single amount string.
    #[must_use]
    pub fn amount(amount: impl Into<String>) -> Self {
        Self {
            amount: Some(amount.into()),
        }
    }
}

/// Failure resolving a settlement override amount string.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum SettlementOverrideError {
    /// Authorized maximum is not a base-10 integer.
    #[error("invalid requirements amount: {0}")]
    InvalidRequirementsAmount(String),
    /// Percent or dollar override failed to parse / convert.
    #[error("invalid settlement override amount: {0}")]
    InvalidOverride(String),
    /// Intermediate arithmetic overflowed.
    #[error("settlement override arithmetic overflow")]
    Overflow,
}

/// Default ERC-20 style decimals when `extra.decimals` is absent (matches Go).
pub const DEFAULT_ASSET_DECIMALS: u32 = 6;

/// Reads `decimals` from payment-requirements `extra` (number or numeric string).
///
/// Falls back to [`DEFAULT_ASSET_DECIMALS`].
#[must_use]
pub fn asset_decimals_from_extra(extra: Option<&serde_json::Value>) -> u32 {
    let Some(extra) = extra else {
        return DEFAULT_ASSET_DECIMALS;
    };
    match extra.get("decimals") {
        Some(serde_json::Value::Number(n)) => n
            .as_u64()
            .and_then(|v| u32::try_from(v).ok())
            .unwrap_or(DEFAULT_ASSET_DECIMALS),
        Some(serde_json::Value::String(s)) => s.parse().unwrap_or(DEFAULT_ASSET_DECIMALS),
        _ => DEFAULT_ASSET_DECIMALS,
    }
}

/// Resolves a settlement override amount to atomic units (decimal string).
///
/// # Errors
///
/// Returns [`SettlementOverrideError`] when the override or authorized max
/// cannot be parsed, or intermediate arithmetic overflows `u128`.
pub fn resolve_settlement_override_amount(
    raw_amount: &str,
    authorized_max: &str,
    decimals: u32,
) -> Result<String, SettlementOverrideError> {
    let raw = raw_amount.trim();
    if let Some(percent_body) = raw.strip_suffix('%') {
        return resolve_percent(percent_body.trim(), authorized_max);
    }
    if let Some(dollar_body) = raw.strip_prefix('$') {
        return resolve_dollar(dollar_body.trim(), decimals);
    }
    // Raw atomic units: pass through after validating base-10 digits.
    if raw.is_empty() || !raw.bytes().all(|b| b.is_ascii_digit()) {
        return Err(SettlementOverrideError::InvalidOverride(raw.to_owned()));
    }
    Ok(raw.to_owned())
}

fn resolve_percent(percent: &str, authorized_max: &str) -> Result<String, SettlementOverrideError> {
    // Up to 2 decimal places: "50" / "50.5" / "50.25" → scaled hundredths of a percent.
    let (int_part, frac_part) = match percent.split_once('.') {
        Some((i, f)) => (i, f),
        None => (percent, ""),
    };
    if int_part.is_empty()
        || !int_part.bytes().all(|b| b.is_ascii_digit())
        || frac_part.len() > 2
        || !frac_part.bytes().all(|b| b.is_ascii_digit())
    {
        return Err(SettlementOverrideError::InvalidOverride(format!(
            "{percent}%"
        )));
    }
    let int_val: u128 = int_part
        .parse()
        .map_err(|_| SettlementOverrideError::InvalidOverride(format!("{percent}%")))?;
    let frac_padded = format!("{frac_part:0<2}");
    let frac_val: u128 = if frac_padded.is_empty() {
        0
    } else {
        frac_padded
            .parse()
            .map_err(|_| SettlementOverrideError::InvalidOverride(format!("{percent}%")))?
    };
    // scaledPercent = int*100 + frac  (percent with 2 d.p. × 100)
    let scaled_percent = int_val
        .checked_mul(100)
        .and_then(|v| v.checked_add(frac_val))
        .ok_or(SettlementOverrideError::Overflow)?;

    let base = parse_u128_amount(authorized_max)?;
    // result = base * scaledPercent / 10000
    let product = base
        .checked_mul(scaled_percent)
        .ok_or(SettlementOverrideError::Overflow)?;
    Ok((product / 10_000).to_string())
}

fn resolve_dollar(dollar: &str, decimals: u32) -> Result<String, SettlementOverrideError> {
    let dollar_dec = Decimal::from_str(dollar)
        .map_err(|_| SettlementOverrideError::InvalidOverride(format!("${dollar}")))?;
    if dollar_dec.is_sign_negative() {
        return Err(SettlementOverrideError::InvalidOverride(format!(
            "${dollar}"
        )));
    }
    let scale = Decimal::from(10u64.pow(decimals.min(18)));
    let atomic = dollar_dec
        .checked_mul(scale)
        .ok_or(SettlementOverrideError::Overflow)?;
    // Floor toward zero for positive values (matches Go big.Float.Int).
    let truncated = atomic.trunc();
    let s = truncated.normalize().to_string();
    // Decimal may emit "1000.0" — strip fractional part if present after trunc.
    Ok(s.split('.').next().unwrap_or("0").to_owned())
}

fn parse_u128_amount(raw: &str) -> Result<u128, SettlementOverrideError> {
    if raw.is_empty() || !raw.bytes().all(|b| b.is_ascii_digit()) {
        return Err(SettlementOverrideError::InvalidRequirementsAmount(
            raw.to_owned(),
        ));
    }
    raw.parse()
        .map_err(|_| SettlementOverrideError::InvalidRequirementsAmount(raw.to_owned()))
}

#[cfg(test)]
mod override_tests {
    use super::*;

    #[test]
    fn atomic_passthrough() {
        assert_eq!(
            resolve_settlement_override_amount("500", "1000000", 6).unwrap(),
            "500"
        );
    }

    #[test]
    fn percent_half() {
        assert_eq!(
            resolve_settlement_override_amount("50%", "1000000", 6).unwrap(),
            "500000"
        );
    }

    #[test]
    fn percent_with_fraction() {
        // 12.5% of 1000 = 125
        assert_eq!(
            resolve_settlement_override_amount("12.5%", "1000", 6).unwrap(),
            "125"
        );
    }

    #[test]
    fn dollar_default_decimals() {
        assert_eq!(
            resolve_settlement_override_amount("$0.001", "1000000", 6).unwrap(),
            "1000"
        );
    }

    #[test]
    fn rejects_empty_atomic() {
        assert!(resolve_settlement_override_amount("", "1", 6).is_err());
    }

    #[test]
    fn decimals_from_extra() {
        let v = serde_json::json!({"decimals": 18});
        assert_eq!(asset_decimals_from_extra(Some(&v)), 18);
        assert_eq!(asset_decimals_from_extra(None), 6);
    }
}
