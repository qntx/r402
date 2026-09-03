//! Machine-readable wire codes (x402 v2 spec §9).

use std::fmt::{self, Display, Formatter};

use compact_str::CompactString;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// Canonical error code on the wire when a payment fails.
///
/// Unknown codes round-trip through [`Self::Custom`].
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ErrorReason {
    /// Wire code: `insufficient_funds`.
    InsufficientFunds,
    /// Wire code: `invalid_exact_evm_payload_authorization_valid_after`.
    InvalidExactEvmPayloadAuthorizationValidAfter,
    /// Wire code: `invalid_exact_evm_payload_authorization_valid_before`.
    InvalidExactEvmPayloadAuthorizationValidBefore,
    /// Wire code: `invalid_exact_evm_payload_authorization_value_mismatch`.
    InvalidExactEvmPayloadAuthorizationValueMismatch,
    /// Wire code: `invalid_exact_evm_payload_signature`.
    InvalidExactEvmPayloadSignature,
    /// Wire code: `invalid_exact_evm_payload_recipient_mismatch`.
    InvalidExactEvmPayloadRecipientMismatch,
    /// Wire code: `invalid_network`.
    InvalidNetwork,
    /// Wire code: `invalid_payload`.
    InvalidPayload,
    /// Wire code: `invalid_payment_requirements`.
    InvalidPaymentRequirements,
    /// Wire code: `invalid_scheme`.
    InvalidScheme,
    /// Wire code: `unsupported_scheme`.
    UnsupportedScheme,
    /// Wire code: `invalid_x402_version`.
    InvalidX402Version,
    /// Wire code: `invalid_transaction_state`.
    InvalidTransactionState,
    /// Wire code: `unexpected_verify_error`.
    UnexpectedVerifyError,
    /// Wire code: `unexpected_settle_error`.
    UnexpectedSettleError,
    /// Wire code: `settlement_pending`.
    ///
    /// Non-terminal. A [`crate::payment::SettleResponse::Failure`] with this
    /// reason MUST carry a non-empty `transaction`.
    SettlementPending,
    /// Wire code: `duplicate_settlement`.
    DuplicateSettlement,
    /// Wire code: `nonce_already_used`.
    NonceAlreadyUsed,
    /// Wire code: `permit2_allowance_required`. HTTP mapping: 412.
    Permit2AllowanceRequired,
    /// Wire code: `invalid_upto_evm_payload_settlement_exceeds_amount`.
    InvalidUptoEvmPayloadSettlementExceedsAmount,
    /// Wire code: `upto_facilitator_mismatch`.
    UptoFacilitatorMismatch,
    /// Wire code: `upto_unauthorized_facilitator`.
    UptoUnauthorizedFacilitator,
    /// Wire code: `upto_amount_exceeds_permitted`.
    UptoAmountExceedsPermitted,
    /// Wire code: `invalid_exact_solana_payload_memo_mismatch`.
    InvalidExactSolanaPayloadMemoMismatch,
    /// Wire code: `invalid_exact_solana_payload_memo_count`.
    InvalidExactSolanaPayloadMemoCount,
    /// Wire code: `incompatible_settlement_mode`. HTTP mapping: 500.
    IncompatibleSettlementMode,
    /// Wire code: `settlement_aborted`. HTTP mapping: 402.
    SettlementAborted,
    /// Wire code: `extension_echo_mismatch`.
    ExtensionEchoMismatch,
    /// Unknown wire code, preserved verbatim.
    Custom(CompactString),
}

impl ErrorReason {
    /// Returns the wire-format code (`snake_case` per spec §9).
    #[must_use]
    pub fn as_str(&self) -> &str {
        match self {
            Self::InsufficientFunds => "insufficient_funds",
            Self::InvalidExactEvmPayloadAuthorizationValidAfter => {
                "invalid_exact_evm_payload_authorization_valid_after"
            }
            Self::InvalidExactEvmPayloadAuthorizationValidBefore => {
                "invalid_exact_evm_payload_authorization_valid_before"
            }
            Self::InvalidExactEvmPayloadAuthorizationValueMismatch => {
                "invalid_exact_evm_payload_authorization_value_mismatch"
            }
            Self::InvalidExactEvmPayloadSignature => "invalid_exact_evm_payload_signature",
            Self::InvalidExactEvmPayloadRecipientMismatch => {
                "invalid_exact_evm_payload_recipient_mismatch"
            }
            Self::InvalidNetwork => "invalid_network",
            Self::InvalidPayload => "invalid_payload",
            Self::InvalidPaymentRequirements => "invalid_payment_requirements",
            Self::InvalidScheme => "invalid_scheme",
            Self::UnsupportedScheme => "unsupported_scheme",
            Self::InvalidX402Version => "invalid_x402_version",
            Self::InvalidTransactionState => "invalid_transaction_state",
            Self::UnexpectedVerifyError => "unexpected_verify_error",
            Self::UnexpectedSettleError => "unexpected_settle_error",
            Self::SettlementPending => "settlement_pending",
            Self::DuplicateSettlement => "duplicate_settlement",
            Self::NonceAlreadyUsed => "nonce_already_used",
            Self::Permit2AllowanceRequired => "permit2_allowance_required",
            Self::InvalidUptoEvmPayloadSettlementExceedsAmount => {
                "invalid_upto_evm_payload_settlement_exceeds_amount"
            }
            Self::UptoFacilitatorMismatch => "upto_facilitator_mismatch",
            Self::UptoUnauthorizedFacilitator => "upto_unauthorized_facilitator",
            Self::UptoAmountExceedsPermitted => "upto_amount_exceeds_permitted",
            Self::InvalidExactSolanaPayloadMemoMismatch => {
                "invalid_exact_solana_payload_memo_mismatch"
            }
            Self::InvalidExactSolanaPayloadMemoCount => "invalid_exact_solana_payload_memo_count",
            Self::IncompatibleSettlementMode => "incompatible_settlement_mode",
            Self::SettlementAborted => "settlement_aborted",
            Self::ExtensionEchoMismatch => "extension_echo_mismatch",
            Self::Custom(s) => s.as_str(),
        }
    }

    /// Maps a wire code to a canonical variant, or [`Self::Custom`].
    #[must_use]
    pub fn from_wire(code: &str) -> Self {
        match code {
            "insufficient_funds" => Self::InsufficientFunds,
            "invalid_exact_evm_payload_authorization_valid_after" => {
                Self::InvalidExactEvmPayloadAuthorizationValidAfter
            }
            "invalid_exact_evm_payload_authorization_valid_before" => {
                Self::InvalidExactEvmPayloadAuthorizationValidBefore
            }
            "invalid_exact_evm_payload_authorization_value_mismatch" => {
                Self::InvalidExactEvmPayloadAuthorizationValueMismatch
            }
            "invalid_exact_evm_payload_signature" => Self::InvalidExactEvmPayloadSignature,
            "invalid_exact_evm_payload_recipient_mismatch" => {
                Self::InvalidExactEvmPayloadRecipientMismatch
            }
            "invalid_network" => Self::InvalidNetwork,
            "invalid_payload" => Self::InvalidPayload,
            "invalid_payment_requirements" => Self::InvalidPaymentRequirements,
            "invalid_scheme" => Self::InvalidScheme,
            "unsupported_scheme" => Self::UnsupportedScheme,
            "invalid_x402_version" => Self::InvalidX402Version,
            "invalid_transaction_state" => Self::InvalidTransactionState,
            "unexpected_verify_error" => Self::UnexpectedVerifyError,
            "unexpected_settle_error" => Self::UnexpectedSettleError,
            "settlement_pending" => Self::SettlementPending,
            "duplicate_settlement" => Self::DuplicateSettlement,
            "nonce_already_used" => Self::NonceAlreadyUsed,
            "permit2_allowance_required" => Self::Permit2AllowanceRequired,
            "invalid_upto_evm_payload_settlement_exceeds_amount" => {
                Self::InvalidUptoEvmPayloadSettlementExceedsAmount
            }
            "upto_facilitator_mismatch" => Self::UptoFacilitatorMismatch,
            "upto_unauthorized_facilitator" => Self::UptoUnauthorizedFacilitator,
            "upto_amount_exceeds_permitted" => Self::UptoAmountExceedsPermitted,
            "invalid_exact_solana_payload_memo_mismatch" => {
                Self::InvalidExactSolanaPayloadMemoMismatch
            }
            "invalid_exact_solana_payload_memo_count" => Self::InvalidExactSolanaPayloadMemoCount,
            "incompatible_settlement_mode" => Self::IncompatibleSettlementMode,
            "settlement_aborted" => Self::SettlementAborted,
            "extension_echo_mismatch" => Self::ExtensionEchoMismatch,
            other => Self::Custom(CompactString::from(other)),
        }
    }
}

impl Display for ErrorReason {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl Serialize for ErrorReason {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for ErrorReason {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = CompactString::deserialize(deserializer)?;
        Ok(Self::from_wire(&s))
    }
}

impl From<&str> for ErrorReason {
    fn from(value: &str) -> Self {
        Self::from_wire(value)
    }
}

impl From<CompactString> for ErrorReason {
    fn from(value: CompactString) -> Self {
        Self::from_wire(&value)
    }
}

impl From<String> for ErrorReason {
    fn from(value: String) -> Self {
        Self::from_wire(&value)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, reason = "unit tests panic on assertion failure")]
mod tests {
    use super::ErrorReason;

    #[test]
    fn settlement_pending_wire_code() {
        assert_eq!(
            ErrorReason::SettlementPending.as_str(),
            "settlement_pending"
        );
        assert_eq!(
            ErrorReason::from_wire("settlement_pending"),
            ErrorReason::SettlementPending
        );
        let json = serde_json::to_value(ErrorReason::SettlementPending).unwrap();
        assert_eq!(json, serde_json::json!("settlement_pending"));
        let back: ErrorReason = serde_json::from_value(json).unwrap();
        assert_eq!(back, ErrorReason::SettlementPending);
    }

    #[test]
    fn incompatible_mode_and_aborted_roundtrip() {
        assert_eq!(
            ErrorReason::from_wire("incompatible_settlement_mode"),
            ErrorReason::IncompatibleSettlementMode
        );
        assert_eq!(
            ErrorReason::from_wire("settlement_aborted"),
            ErrorReason::SettlementAborted
        );
        assert_eq!(
            ErrorReason::IncompatibleSettlementMode.as_str(),
            "incompatible_settlement_mode"
        );
        assert_eq!(
            ErrorReason::SettlementAborted.as_str(),
            "settlement_aborted"
        );
        assert_eq!(
            ErrorReason::from_wire("extension_echo_mismatch"),
            ErrorReason::ExtensionEchoMismatch
        );
        assert_eq!(
            ErrorReason::ExtensionEchoMismatch.as_str(),
            "extension_echo_mismatch"
        );
    }

    #[test]
    fn custom_preserves_unknown_code() {
        let reason = ErrorReason::from_wire("vendor_specific_diagnostic_42");
        assert_eq!(reason.as_str(), "vendor_specific_diagnostic_42");
        assert_eq!(
            reason,
            ErrorReason::Custom(compact_str::CompactString::from(
                "vendor_specific_diagnostic_42"
            ))
        );
    }
}
