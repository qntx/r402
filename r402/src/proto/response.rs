//! Verify and settle response types.
//!
//! Responses returned by a facilitator after processing verification or
//! settlement requests. Each response uses a flat wire format with a boolean
//! discriminator, converted to/from a Rust enum via serde.

use serde::{Deserialize, Serialize};

use super::{AsPaymentProblem, Base64Bytes, Extensions};

/// Result returned by a facilitator after verifying a payment payload
/// against the provided payment requirements.
///
/// This response indicates whether the payment authorization is valid and identifies
/// the payer. If invalid, it includes a reason describing why verification failed
/// (e.g., wrong network, invalid scheme, insufficient funds).
///
/// # Examples
///
/// ```
/// use r402::proto::VerifyResponse;
///
/// let valid = VerifyResponse::valid("0xAbC123".to_string());
/// assert!(valid.is_valid());
///
/// let invalid = VerifyResponse::invalid(None, "expired".to_string());
/// assert!(!invalid.is_valid());
///
/// let json = serde_json::to_string(&valid).unwrap();
/// assert!(json.contains("\"isValid\":true"));
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(into = "VerifyResponseWire", try_from = "VerifyResponseWire")]
#[non_exhaustive]
pub enum VerifyResponse {
    /// The payload matches the requirements and passes all checks.
    Valid {
        /// The address of the payer.
        payer: String,
    },
    /// The payload was well-formed but failed verification.
    Invalid {
        /// Machine-readable reason verification failed.
        reason: String,
        /// Optional human-readable description of the failure.
        message: Option<String>,
        /// The payer address, if identifiable.
        payer: Option<String>,
    },
}

impl VerifyResponse {
    /// Constructs a successful verification response with the given payer address.
    #[must_use]
    pub const fn valid(payer: String) -> Self {
        Self::Valid { payer }
    }

    /// Constructs a failed verification response.
    #[must_use]
    pub const fn invalid(payer: Option<String>, reason: String) -> Self {
        Self::Invalid {
            reason,
            message: None,
            payer,
        }
    }

    /// Constructs a failed verification response with a human-readable message.
    #[must_use]
    pub const fn invalid_with_message(
        payer: Option<String>,
        reason: String,
        message: String,
    ) -> Self {
        Self::Invalid {
            reason,
            message: Some(message),
            payer,
        }
    }

    /// Returns `true` if the verification succeeded.
    #[must_use]
    pub const fn is_valid(&self) -> bool {
        matches!(self, Self::Valid { .. })
    }

    /// Converts a [`FacilitatorError`] into an invalid verification response,
    /// preserving the structured reason code and message from the error.
    ///
    /// This is the canonical conversion for HTTP boundaries that need to return
    /// a wire-compatible `VerifyResponse` instead of propagating errors.
    ///
    /// [`FacilitatorError`]: crate::facilitator::FacilitatorError
    #[must_use]
    pub fn from_facilitator_error(error: &crate::facilitator::FacilitatorError) -> Self {
        let problem = error.as_payment_problem();
        Self::Invalid {
            reason: problem.reason().to_string(),
            message: Some(problem.details().to_owned()),
            payer: None,
        }
    }
}

/// Wire format for [`VerifyResponse`], using a flat boolean discriminator.
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct VerifyResponseWire {
    is_valid: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    payer: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    invalid_reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    invalid_message: Option<String>,
}

impl From<VerifyResponse> for VerifyResponseWire {
    fn from(resp: VerifyResponse) -> Self {
        match resp {
            VerifyResponse::Valid { payer } => Self {
                is_valid: true,
                payer: Some(payer),
                invalid_reason: None,
                invalid_message: None,
            },
            VerifyResponse::Invalid {
                reason,
                message,
                payer,
            } => Self {
                is_valid: false,
                payer,
                invalid_reason: Some(reason),
                invalid_message: message,
            },
        }
    }
}

impl TryFrom<VerifyResponseWire> for VerifyResponse {
    type Error = String;

    fn try_from(wire: VerifyResponseWire) -> Result<Self, Self::Error> {
        if wire.is_valid {
            let payer = wire.payer.ok_or("missing field: payer")?;
            Ok(Self::Valid { payer })
        } else {
            let reason = wire.invalid_reason.ok_or("missing field: invalidReason")?;
            Ok(Self::Invalid {
                reason,
                message: wire.invalid_message,
                payer: wire.payer,
            })
        }
    }
}

/// Response from a payment settlement request.
///
/// Indicates whether the payment was successfully settled on-chain,
/// including the transaction hash and payer address on success.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(into = "SettleResponseWire", try_from = "SettleResponseWire")]
#[non_exhaustive]
pub enum SettleResponse {
    /// Settlement succeeded.
    Success {
        /// The address that paid.
        payer: String,
        /// The on-chain transaction hash.
        transaction: String,
        /// The network where settlement occurred (CAIP-2 chain ID).
        network: String,
        /// Optional protocol extensions returned by the facilitator.
        extensions: Option<Extensions>,
    },
    /// Settlement failed.
    Error {
        /// Machine-readable reason for failure.
        reason: String,
        /// Optional human-readable description of the failure.
        message: Option<String>,
        /// The payer address, if identifiable.
        payer: Option<String>,
        /// The network where settlement was attempted.
        network: String,
    },
}

impl SettleResponse {
    /// Returns `true` if the settlement succeeded.
    #[must_use]
    pub const fn is_success(&self) -> bool {
        matches!(self, Self::Success { .. })
    }

    /// Encode a **successful** settlement into base64 bytes suitable for the
    /// `Payment-Response` HTTP header.
    ///
    /// Returns `None` if this is an [`Error`](Self::Error) variant, preventing
    /// accidental encoding of failed settlements into response headers.
    ///
    /// The output bytes are standard base64 (RFC 4648) of the JSON-serialised
    /// [`SettleResponse`] and can be placed directly into a header value.
    #[must_use]
    pub fn encode_base64(&self) -> Option<Base64Bytes> {
        if !self.is_success() {
            return None;
        }
        let json = serde_json::to_vec(self).ok()?;
        Some(Base64Bytes::encode(json))
    }

    /// Converts a [`FacilitatorError`] into a settlement error response,
    /// preserving the structured reason code and message from the error.
    ///
    /// `network` should be the CAIP-2 chain identifier from the original request
    /// (e.g., obtained via [`super::SettleRequest::network()`]).
    ///
    /// [`FacilitatorError`]: crate::facilitator::FacilitatorError
    #[must_use]
    pub fn from_facilitator_error(
        error: &crate::facilitator::FacilitatorError,
        network: String,
    ) -> Self {
        let problem = error.as_payment_problem();
        Self::Error {
            reason: problem.reason().to_string(),
            message: Some(problem.details().to_owned()),
            payer: None,
            network,
        }
    }
}

/// Wire format for [`SettleResponse`], using a flat boolean discriminator.
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SettleResponseWire {
    success: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    error_reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    error_message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    payer: Option<String>,
    #[serde(default)]
    transaction: String,
    network: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    extensions: Option<Extensions>,
}

impl From<SettleResponse> for SettleResponseWire {
    fn from(resp: SettleResponse) -> Self {
        match resp {
            SettleResponse::Success {
                payer,
                transaction,
                network,
                extensions,
            } => Self {
                success: true,
                error_reason: None,
                error_message: None,
                payer: Some(payer),
                transaction,
                network,
                extensions,
            },
            SettleResponse::Error {
                reason,
                message,
                payer,
                network,
            } => Self {
                success: false,
                error_reason: Some(reason),
                error_message: message,
                payer,
                transaction: String::new(),
                network,
                extensions: None,
            },
        }
    }
}

impl TryFrom<SettleResponseWire> for SettleResponse {
    type Error = String;

    fn try_from(
        wire: SettleResponseWire,
    ) -> Result<Self, <Self as TryFrom<SettleResponseWire>>::Error> {
        if wire.success {
            let payer = wire.payer.ok_or("missing field: payer")?;
            if wire.transaction.is_empty() {
                return Err("missing field: transaction".to_owned());
            }
            Ok(Self::Success {
                payer,
                transaction: wire.transaction,
                network: wire.network,
                extensions: wire.extensions,
            })
        } else {
            let reason = wire.error_reason.ok_or("missing field: errorReason")?;
            Ok(Self::Error {
                reason,
                message: wire.error_message,
                payer: wire.payer,
                network: wire.network,
            })
        }
    }
}

#[cfg(test)]
#[allow(
    clippy::indexing_slicing,
    reason = "test assertions on known JSON structure"
)]
mod tests {
    use super::*;

    #[test]
    fn verify_valid_roundtrip() {
        let resp = VerifyResponse::valid("0xABC".into());
        let json = serde_json::to_value(&resp).unwrap();

        assert_eq!(json["isValid"], true);
        assert_eq!(json["payer"], "0xABC");
        assert!(json.get("invalidReason").is_none());

        let back: VerifyResponse = serde_json::from_value(json).unwrap();
        assert!(back.is_valid());
        assert!(matches!(back, VerifyResponse::Valid { payer } if payer == "0xABC"));
    }

    #[test]
    fn verify_invalid_roundtrip() {
        let resp = VerifyResponse::invalid_with_message(
            Some("0xDEF".into()),
            "insufficient_balance".into(),
            "not enough USDC".into(),
        );
        let json = serde_json::to_value(&resp).unwrap();

        assert_eq!(json["isValid"], false);
        assert_eq!(json["invalidReason"], "insufficient_balance");
        assert_eq!(json["invalidMessage"], "not enough USDC");
        assert_eq!(json["payer"], "0xDEF");

        let back: VerifyResponse = serde_json::from_value(json).unwrap();
        assert!(!back.is_valid());
        assert!(matches!(
            back,
            VerifyResponse::Invalid { reason, message, payer }
                if reason == "insufficient_balance"
                && message.as_deref() == Some("not enough USDC")
                && payer.as_deref() == Some("0xDEF")
        ));
    }

    #[test]
    fn verify_valid_missing_payer_rejects() {
        let json = serde_json::json!({"isValid": true});
        let err = serde_json::from_value::<VerifyResponse>(json).unwrap_err();
        assert!(
            err.to_string().contains("payer"),
            "should mention missing payer"
        );
    }

    #[test]
    fn verify_invalid_missing_reason_rejects() {
        let json = serde_json::json!({"isValid": false});
        let err = serde_json::from_value::<VerifyResponse>(json).unwrap_err();
        assert!(
            err.to_string().contains("invalidReason"),
            "should mention missing invalidReason"
        );
    }

    #[test]
    fn settle_success_roundtrip() {
        let resp = SettleResponse::Success {
            payer: "0xABC".into(),
            transaction: "0xTX123".into(),
            network: "eip155:8453".into(),
            extensions: None,
        };
        let json = serde_json::to_value(&resp).unwrap();

        assert_eq!(json["success"], true);
        assert_eq!(json["payer"], "0xABC");
        assert_eq!(json["transaction"], "0xTX123");
        assert_eq!(json["network"], "eip155:8453");
        assert!(json.get("errorReason").is_none());

        let back: SettleResponse = serde_json::from_value(json).unwrap();
        assert!(back.is_success());
    }

    #[test]
    fn settle_error_roundtrip() {
        let resp = SettleResponse::Error {
            reason: "tx_reverted".into(),
            message: Some("out of gas".into()),
            payer: None,
            network: "eip155:1".into(),
        };
        let json = serde_json::to_value(&resp).unwrap();

        assert_eq!(json["success"], false);
        assert_eq!(json["errorReason"], "tx_reverted");
        assert_eq!(json["errorMessage"], "out of gas");
        assert_eq!(json["network"], "eip155:1");

        let back: SettleResponse = serde_json::from_value(json).unwrap();
        assert!(!back.is_success());
    }

    #[test]
    fn settle_success_missing_payer_rejects() {
        let json = serde_json::json!({
            "success": true,
            "transaction": "0xTX",
            "network": "eip155:1"
        });
        let err = serde_json::from_value::<SettleResponse>(json).unwrap_err();
        assert!(err.to_string().contains("payer"));
    }

    #[test]
    fn settle_success_empty_tx_rejects() {
        let json = serde_json::json!({
            "success": true,
            "payer": "0xABC",
            "transaction": "",
            "network": "eip155:1"
        });
        let err = serde_json::from_value::<SettleResponse>(json).unwrap_err();
        assert!(err.to_string().contains("transaction"));
    }

    #[test]
    fn settle_encode_base64_only_for_success() {
        let success = SettleResponse::Success {
            payer: "0xA".into(),
            transaction: "0xT".into(),
            network: "eip155:1".into(),
            extensions: None,
        };
        assert!(success.encode_base64().is_some());

        let error = SettleResponse::Error {
            reason: "fail".into(),
            message: None,
            payer: None,
            network: "eip155:1".into(),
        };
        assert!(error.encode_base64().is_none());
    }
}
