//! Verify and settle request envelopes.

use std::str::FromStr;

use serde::{Deserialize, Serialize};

use super::version::Version2;
use crate::chain::ChainId;
use crate::error::VerificationError;
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
    pub x402_version: super::Version<V>,
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
mod tests {
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
