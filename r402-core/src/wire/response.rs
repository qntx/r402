//! Verify and settle response bodies.
//!
//! Both responses use a boolean discriminator on the wire (`isValid` /
//! `success`) and convert to/from a Rust enum via a private "wire" struct.
//! Successful responses may now carry an `extensions` block returned by the
//! facilitator; settlement success also carries the actual amount settled
//! (required by the `upto` scheme).

use compact_str::CompactString;
use serde::{Deserialize, Serialize};

use super::{Base64Bytes, Extensions};
use crate::error::FacilitatorError;
use crate::error_reason::{AsPaymentProblem, ErrorReason};

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
#[serde(rename_all = "camelCase")]
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
    /// transmitting a failed settlement as if it were successful.
    #[must_use]
    pub fn encode_base64(&self) -> Option<Base64Bytes> {
        if !self.is_success() {
            return None;
        }
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
#[serde(rename_all = "camelCase")]
struct SettleResponseWire {
    success: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    error_reason: Option<ErrorReason>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    error_message: Option<CompactString>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    payer: Option<CompactString>,
    /// Always serialized per x402 v2 spec §5.3.2: empty string on failure,
    /// transaction hash on success. Deserialization tolerates omission for
    /// legacy clients but the success path enforces non-empty in `try_from`.
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
            if wire.transaction.is_empty() {
                return Err("missing field: transaction".to_owned());
            }
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
mod tests {
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
    fn settle_success_missing_transaction_rejects() {
        let json = json!({
            "success": true,
            "payer": "0xABC",
            "transaction": "",
            "network": "eip155:1"
        });
        assert!(serde_json::from_value::<SettleResponse>(json).is_err());
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
