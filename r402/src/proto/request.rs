//! Verify and settle request types.
//!
//! Wrappers around raw JSON payloads that provide type-safe access to
//! payment verification and settlement requests.

use std::str::FromStr;

use serde::{Deserialize, Serialize};

use super::{PaymentVerificationError, v2};
use crate::chain::ChainId;
use crate::scheme::SchemeSlug;

/// A version-tagged verify/settle request with typed payload and requirements.
///
/// The const parameter `V` selects the protocol version marker ([`super::Version<V>`]).
///
/// Use [`v2::VerifyRequest`] type alias instead of constructing this directly.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TypedVerifyRequest<const V: u8, TPayload, TRequirements> {
    /// The protocol version marker.
    pub x402_version: super::Version<V>,
    /// The signed payment authorization.
    pub payment_payload: TPayload,
    /// The payment requirements to verify against.
    pub payment_requirements: TRequirements,
}

impl<const V: u8, TPayload, TRequirements> TypedVerifyRequest<V, TPayload, TRequirements>
where
    Self: serde::de::DeserializeOwned,
{
    /// Deserializes from a protocol-level [`VerifyRequest`].
    ///
    /// # Errors
    ///
    /// Returns [`PaymentVerificationError`] if deserialization fails.
    pub fn from_proto(request: VerifyRequest) -> Result<Self, PaymentVerificationError> {
        let deserialized: Self = serde_json::from_value(request.into_json())?;
        Ok(deserialized)
    }

    /// Deserializes from a protocol-level [`SettleRequest`].
    ///
    /// Settlement reuses the same wire format as verification.
    ///
    /// # Errors
    ///
    /// Returns [`PaymentVerificationError`] if deserialization fails.
    pub fn from_settle(request: SettleRequest) -> Result<Self, PaymentVerificationError> {
        let deserialized: Self = serde_json::from_value(request.into_json())?;
        Ok(deserialized)
    }
}

impl<const V: u8, TPayload, TRequirements> TryInto<VerifyRequest>
    for TypedVerifyRequest<V, TPayload, TRequirements>
where
    TPayload: Serialize,
    TRequirements: Serialize,
{
    type Error = serde_json::Error;
    fn try_into(self) -> Result<VerifyRequest, Self::Error> {
        let json = serde_json::to_value(self)?;
        Ok(VerifyRequest(json))
    }
}

/// Request to verify a payment before settlement.
///
/// This wrapper contains the payment payload and requirements sent by a client
/// to a facilitator for verification. The facilitator checks that the payment
/// authorization is valid, properly signed, and matches the requirements.
///
/// The inner JSON structure varies by protocol version and scheme.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerifyRequest(serde_json::Value);

impl From<serde_json::Value> for VerifyRequest {
    fn from(value: serde_json::Value) -> Self {
        Self(value)
    }
}

impl VerifyRequest {
    /// Consumes the request and returns the inner JSON value.
    #[must_use]
    pub fn into_json(self) -> serde_json::Value {
        self.0
    }

    /// Extracts the scheme handler slug from the request.
    ///
    /// This determines which scheme handler should process this payment
    /// based on the chain ID and scheme name.
    ///
    /// Returns `None` if the request format is invalid or the scheme is unknown.
    #[must_use]
    pub fn scheme_slug(&self) -> Option<SchemeSlug> {
        scheme_slug_from_json(&self.0)
    }
}

/// Request to settle a verified payment on-chain.
///
/// Structurally identical to [`VerifyRequest`] on the wire, but represented as a
/// distinct type so the compiler can prevent accidental misuse (e.g., passing a
/// verify request where a settle request is expected).
///
/// Use `From<VerifyRequest>` to convert a verified request into a settle request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SettleRequest(serde_json::Value);

impl SettleRequest {
    /// Consumes the request and returns the inner JSON value.
    #[must_use]
    pub fn into_json(self) -> serde_json::Value {
        self.0
    }

    /// Extracts the scheme handler slug from the request.
    ///
    /// Delegates to the same logic as [`VerifyRequest::scheme_slug`].
    #[must_use]
    pub fn scheme_slug(&self) -> Option<SchemeSlug> {
        scheme_slug_from_json(&self.0)
    }

    /// Returns the CAIP-2 network identifier from `paymentRequirements.network`.
    ///
    /// Returns an empty string if the field is absent or not a string.
    #[must_use]
    pub fn network(&self) -> &str {
        self.0
            .get("paymentRequirements")
            .and_then(|r| r.get("network"))
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
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

/// Extracts a [`SchemeSlug`] from a raw verify/settle JSON value.
///
/// Navigates `x402Version`, `paymentPayload.accepted.network`, and
/// `paymentPayload.accepted.scheme` without cloning the JSON tree.
fn scheme_slug_from_json(json: &serde_json::Value) -> Option<SchemeSlug> {
    let x402_version: u8 = json.get("x402Version")?.as_u64()?.try_into().ok()?;
    if x402_version != v2::Version2::VALUE {
        return None;
    }
    let accepted = json.get("paymentPayload")?.get("accepted")?;
    let chain_id = ChainId::from_str(accepted.get("network")?.as_str()?).ok()?;
    let scheme = accepted.get("scheme")?.as_str()?;
    Some(SchemeSlug::new(chain_id, scheme.into()))
}

#[cfg(test)]
#[allow(
    clippy::indexing_slicing,
    reason = "test mutations on known JSON structure"
)]
mod tests {
    use super::*;

    fn make_v2_json(network: &str, scheme: &str) -> serde_json::Value {
        serde_json::json!({
            "x402Version": 2,
            "paymentPayload": {
                "accepted": {
                    "network": network,
                    "scheme": scheme
                }
            },
            "paymentRequirements": {
                "network": network
            }
        })
    }

    #[test]
    fn slug_extracts_evm_exact() {
        let json = make_v2_json("eip155:8453", "exact");
        let slug = scheme_slug_from_json(&json).unwrap();
        assert_eq!(slug.chain_id, ChainId::new("eip155", "8453"));
        assert_eq!(slug.name, "exact");
    }

    #[test]
    fn slug_extracts_solana_exact() {
        let json = make_v2_json("solana:5eykt4UsFv8P8NJdTREpY1vzqKqZKvdp", "exact");
        let slug = scheme_slug_from_json(&json).unwrap();
        assert_eq!(slug.chain_id.namespace(), "solana");
        assert_eq!(slug.name, "exact");
    }

    #[test]
    fn slug_rejects_wrong_version() {
        let mut json = make_v2_json("eip155:1", "exact");
        json["x402Version"] = serde_json::json!(99);
        assert!(scheme_slug_from_json(&json).is_none());
    }

    #[test]
    fn slug_rejects_missing_payload() {
        let json = serde_json::json!({"x402Version": 2});
        assert!(scheme_slug_from_json(&json).is_none());
    }

    #[test]
    fn slug_rejects_missing_network() {
        let json = serde_json::json!({
            "x402Version": 2,
            "paymentPayload": {"accepted": {"scheme": "exact"}}
        });
        assert!(scheme_slug_from_json(&json).is_none());
    }

    #[test]
    fn slug_rejects_invalid_caip2() {
        let json = make_v2_json("not-a-caip2", "exact");
        assert!(scheme_slug_from_json(&json).is_none());
    }

    #[test]
    fn verify_request_scheme_slug() {
        let json = make_v2_json("eip155:1", "exact");
        let req = VerifyRequest::from(json);
        let slug = req.scheme_slug().unwrap();
        assert_eq!(slug.to_string(), "eip155:1:exact");
    }

    #[test]
    fn settle_request_from_verify_preserves_slug() {
        let json = make_v2_json("eip155:42161", "exact");
        let verify = VerifyRequest::from(json);
        let settle: SettleRequest = verify.into();
        let slug = settle.scheme_slug().unwrap();
        assert_eq!(slug.to_string(), "eip155:42161:exact");
    }

    #[test]
    fn settle_request_network() {
        let json = make_v2_json("eip155:8453", "exact");
        let settle = SettleRequest::from(json);
        assert_eq!(settle.network(), "eip155:8453");
    }

    #[test]
    fn settle_request_network_missing_returns_empty() {
        let settle = SettleRequest::from(serde_json::json!({}));
        assert_eq!(settle.network(), "");
    }
}
