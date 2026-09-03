//! Facilitator `/verify` request and response.

use compact_str::CompactString;
use serde::{Deserialize, Serialize};

use super::{Extensions, Version};
use crate::error::{ErrorReason, FacilitatorError, VerificationError};
use crate::scheme::SchemeSlug;

/// Protocol-versioned verify request parameterized by payload and requirements.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TypedVerifyRequest<const V: u8, TPayload, TRequirements> {
    /// Protocol version marker.
    pub x402_version: Version<V>,
    /// Signed payment authorization.
    pub payment_payload: TPayload,
    /// Payment terms being verified.
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

    /// Inspects the request for scheme routing without full decoding.
    #[must_use]
    pub fn scheme_slug(&self) -> Option<SchemeSlug> {
        super::scheme_slug_from_json(&self.0)
    }

    /// CAIP-2 network identifier from `paymentRequirements.network`.
    #[must_use]
    pub fn network(&self) -> &str {
        super::network_from_json(&self.0)
    }
}

impl From<serde_json::Value> for VerifyRequest {
    fn from(value: serde_json::Value) -> Self {
        Self(value)
    }
}

/// Verification outcome returned by a facilitator.
///
/// `extension_responses` is the `EXTENSION-RESPONSES` sidechannel. It is not
/// serialized into JSON and is never merged into `extensions`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(into = "VerifyResponseWire", try_from = "VerifyResponseWire")]
#[non_exhaustive]
pub enum VerifyResponse {
    /// Payload passed all verification checks.
    Valid {
        /// Address of the verified payer, if the facilitator included it.
        payer: Option<CompactString>,
        /// Facilitator-attached extension data (JSON body).
        extensions: Extensions,
        /// Facilitator `EXTENSION-RESPONSES` sidechannel. Not buyer wire.
        extension_responses: Extensions,
        /// Scheme-specific additional data.
        extra: Option<serde_json::Value>,
    },
    /// Payload was well-formed but failed verification.
    Invalid {
        /// Wire-level reason code, if the facilitator included it.
        reason: Option<ErrorReason>,
        /// Optional human-readable message.
        message: Option<CompactString>,
        /// Optional payer address if identifiable from the payload.
        payer: Option<CompactString>,
        /// Facilitator-attached extension data (JSON body).
        extensions: Extensions,
        /// Facilitator `EXTENSION-RESPONSES` sidechannel. Not buyer wire.
        extension_responses: Extensions,
        /// Scheme-specific additional data.
        extra: Option<serde_json::Value>,
    },
}

impl VerifyResponse {
    /// `Valid` response with empty extensions.
    #[must_use]
    pub fn valid(payer: impl Into<CompactString>) -> Self {
        Self::Valid {
            payer: Some(payer.into()),
            extensions: Extensions::new(),
            extension_responses: Extensions::new(),
            extra: None,
        }
    }

    /// `Invalid` response without a human message.
    #[must_use]
    pub fn invalid(payer: Option<CompactString>, reason: ErrorReason) -> Self {
        Self::Invalid {
            reason: Some(reason),
            message: None,
            payer,
            extensions: Extensions::new(),
            extension_responses: Extensions::new(),
            extra: None,
        }
    }

    /// `Invalid` response with a message.
    #[must_use]
    pub fn invalid_with_message(
        payer: Option<CompactString>,
        reason: ErrorReason,
        message: impl Into<CompactString>,
    ) -> Self {
        Self::Invalid {
            reason: Some(reason),
            message: Some(message.into()),
            payer,
            extensions: Extensions::new(),
            extension_responses: Extensions::new(),
            extra: None,
        }
    }

    /// Whether this is a `Valid` outcome.
    #[must_use]
    pub const fn is_valid(&self) -> bool {
        matches!(self, Self::Valid { .. })
    }

    /// Sidechannel map (empty when the header was absent).
    #[must_use]
    pub const fn extension_responses(&self) -> &Extensions {
        match self {
            Self::Valid {
                extension_responses,
                ..
            }
            | Self::Invalid {
                extension_responses,
                ..
            } => extension_responses,
        }
    }

    /// Replaces the sidechannel map.
    pub fn set_extension_responses(&mut self, responses: Extensions) {
        match self {
            Self::Valid {
                extension_responses,
                ..
            }
            | Self::Invalid {
                extension_responses,
                ..
            } => *extension_responses = responses,
        }
    }

    /// Converts a [`FacilitatorError`] into an `Invalid` response.
    ///
    /// Returns `None` for [`FacilitatorError::Transport`]: that is HTTP 502,
    /// not a 402 body.
    #[must_use]
    pub fn from_facilitator_error(error: &FacilitatorError) -> Option<Self> {
        let problem = error.as_payment_problem()?;
        Some(Self::Invalid {
            reason: Some(problem.reason()),
            message: Some(CompactString::from(problem.details())),
            payer: None,
            extensions: Extensions::new(),
            extension_responses: Extensions::new(),
            extra: None,
        })
    }
}

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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    extra: Option<serde_json::Value>,
}

impl From<VerifyResponse> for VerifyResponseWire {
    fn from(value: VerifyResponse) -> Self {
        match value {
            VerifyResponse::Valid {
                payer,
                extensions,
                extra,
                extension_responses: _,
            } => Self {
                is_valid: true,
                payer,
                invalid_reason: None,
                invalid_message: None,
                extensions,
                extra,
            },
            VerifyResponse::Invalid {
                reason,
                message,
                payer,
                extensions,
                extra,
                extension_responses: _,
            } => Self {
                is_valid: false,
                payer,
                invalid_reason: reason,
                invalid_message: message,
                extensions,
                extra,
            },
        }
    }
}

impl TryFrom<VerifyResponseWire> for VerifyResponse {
    type Error = String;
    fn try_from(wire: VerifyResponseWire) -> Result<Self, Self::Error> {
        if wire.is_valid {
            Ok(Self::Valid {
                payer: wire.payer,
                extensions: wire.extensions,
                extension_responses: Extensions::new(),
                extra: wire.extra,
            })
        } else {
            Ok(Self::Invalid {
                reason: wire.invalid_reason,
                message: wire.invalid_message,
                payer: wire.payer,
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
    fn verify_request_scheme_slug_evm() {
        let req = VerifyRequest::from(v2_json("eip155:8453", "exact"));
        let slug = req.scheme_slug().unwrap();
        assert_eq!(slug.to_string(), "eip155:8453:exact");
    }

    #[test]
    fn slug_rejects_wrong_version() {
        let mut json = v2_json("eip155:1", "exact");
        json["x402Version"] = json!(99);
        assert!(super::super::scheme_slug_from_json(&json).is_none());
    }

    #[test]
    fn slug_rejects_invalid_caip2() {
        assert!(super::super::scheme_slug_from_json(&v2_json("not-a-caip2", "exact")).is_none());
    }

    #[test]
    fn verify_valid_roundtrip() {
        let response = VerifyResponse::valid("0xABC");
        let encoded = serde_json::to_value(&response).unwrap();
        assert_eq!(encoded["isValid"], true);
        assert_eq!(encoded["payer"], "0xABC");
        assert!(encoded.get("invalidReason").is_none());
        assert!(encoded.get("extensionResponses").is_none());

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
            payer: Some("0xABC".into()),
            extensions,
            extension_responses: Extensions::new(),
            extra: None,
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
    fn extension_responses_are_not_serialized() {
        let mut response = VerifyResponse::valid("0xABC");
        let mut side = Extensions::new();
        side.insert("bazaar", ExtensionEntry::raw(json!({"status": "success"})));
        response.set_extension_responses(side);
        let encoded = serde_json::to_value(&response).unwrap();
        assert!(encoded.get("extensionResponses").is_none());
        assert!(encoded.get("extensions").is_none());
        assert!(!response.extension_responses().is_empty());
    }

    #[test]
    fn from_facilitator_error_refuses_transport() {
        let err = FacilitatorError::transport(FacilitatorTransportKind::MalformedSuccessBody);
        assert!(VerifyResponse::from_facilitator_error(&err).is_none());
    }

    #[test]
    fn verify_deny_unknown_top_level_fields() {
        let verify_typo = json!({
            "isValid": true,
            "payer": "0xABC",
            "extraTypo": {"k": 1}
        });
        assert!(serde_json::from_value::<VerifyResponse>(verify_typo).is_err());
    }

    #[test]
    fn verify_valid_omitted_payer() {
        let json = json!({ "isValid": true });
        let back: VerifyResponse = serde_json::from_value(json).unwrap();
        assert!(matches!(back, VerifyResponse::Valid { payer: None, .. }));
        let encoded = serde_json::to_value(&back).unwrap();
        assert!(encoded.get("payer").is_none());
    }

    #[test]
    fn verify_invalid_omitted_reason_and_payer() {
        let json = json!({ "isValid": false });
        let back: VerifyResponse = serde_json::from_value(json).unwrap();
        assert!(matches!(
            back,
            VerifyResponse::Invalid {
                reason: None,
                payer: None,
                ..
            }
        ));
        let encoded = serde_json::to_value(&back).unwrap();
        assert!(encoded.get("invalidReason").is_none());
        assert!(encoded.get("payer").is_none());
    }
}
