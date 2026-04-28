//! Machine-readable error codes used in x402 verify/settle responses.
//!
//! Aligned with the x402 v2 specification §9 "Error Handling" and the
//! cross-SDK conventions established by the Go and TypeScript reference
//! implementations. When the wire emits an unknown code it round-trips
//! losslessly via [`ErrorReason::Custom`].
//!
//! # Categories
//!
//! - **Spec §9 standard codes**: 15 canonical reasons every facilitator MUST
//!   recognise (`insufficient_funds`, `invalid_payload`, ...).
//! - **Cross-chain reserved codes**: pragmatic codes used across SDKs but
//!   not enumerated in §9 (`duplicate_settlement`, `nonce_already_used`,
//!   `permit2_allowance_required`).
//! - **Upto scheme codes**: reserved by `schemes/upto/scheme_upto_evm.md`.
//! - **SVM-prefixed codes**: chain-specific diagnostics emitted by the
//!   Solana facilitator.
//! - **`Custom(CompactString)`**: catch-all preserving any unknown code as
//!   received on the wire so clients never lose information.

use std::fmt::{self, Display, Formatter};

use compact_str::CompactString;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// Canonical error code returned on the wire when a payment fails.
///
/// The variants form a closed set of well-known codes the SDK can pattern
/// match against. Any unknown wire code is preserved via [`Self::Custom`]
/// so deserialisation is total and idempotent.
///
/// The enum is `#[non_exhaustive]`: new well-known variants may be added
/// in minor releases without breaking semver.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ErrorReason {
    // Spec §9 standard codes.
    /// Client does not have enough tokens to complete the payment.
    /// Wire code: `insufficient_funds`.
    InsufficientFunds,
    /// Payment authorization is not yet valid (before validAfter timestamp).
    /// Wire code: `invalid_exact_evm_payload_authorization_valid_after`.
    InvalidExactEvmPayloadAuthorizationValidAfter,
    /// Payment authorization has expired (after validBefore timestamp).
    /// Wire code: `invalid_exact_evm_payload_authorization_valid_before`.
    InvalidExactEvmPayloadAuthorizationValidBefore,
    /// Payment amount does not exactly match the required amount.
    /// Wire code: `invalid_exact_evm_payload_authorization_value_mismatch`.
    InvalidExactEvmPayloadAuthorizationValueMismatch,
    /// Payment authorization signature is invalid or improperly signed.
    /// Wire code: `invalid_exact_evm_payload_signature`.
    InvalidExactEvmPayloadSignature,
    /// Recipient address does not match payment requirements.
    /// Wire code: `invalid_exact_evm_payload_recipient_mismatch`.
    InvalidExactEvmPayloadRecipientMismatch,
    /// Specified blockchain network is not supported.
    /// Wire code: `invalid_network`.
    InvalidNetwork,
    /// Payment payload is malformed or contains invalid data.
    /// Wire code: `invalid_payload`.
    InvalidPayload,
    /// Payment requirements object is invalid or malformed.
    /// Wire code: `invalid_payment_requirements`.
    InvalidPaymentRequirements,
    /// Specified payment scheme is not supported.
    /// Wire code: `invalid_scheme`.
    InvalidScheme,
    /// Payment scheme is not supported by the facilitator.
    /// Wire code: `unsupported_scheme`.
    UnsupportedScheme,
    /// Protocol version is not supported.
    /// Wire code: `invalid_x402_version`.
    InvalidX402Version,
    /// Blockchain transaction failed or was rejected.
    /// Wire code: `invalid_transaction_state`.
    InvalidTransactionState,
    /// Unexpected error occurred during payment verification.
    /// Wire code: `unexpected_verify_error`.
    UnexpectedVerifyError,
    /// Unexpected error occurred during payment settlement.
    /// Wire code: `unexpected_settle_error`.
    UnexpectedSettleError,

    // Cross-chain reserved codes (SDK consensus).
    /// Facilitator detected a duplicate settlement attempt and rejected it.
    /// Wire code: `duplicate_settlement`.
    DuplicateSettlement,
    /// EIP-3009 authorization nonce has already been consumed on-chain.
    /// Wire code: `nonce_already_used`.
    NonceAlreadyUsed,
    /// Permit2 allowance is insufficient; the payer must call `approve`
    /// on the token contract pointing at the Permit2 address first.
    /// Wire code: `permit2_allowance_required`.
    /// HTTP mapping: `412 Precondition Failed`.
    Permit2AllowanceRequired,

    // Upto scheme codes.
    /// Resource server requested a settlement amount exceeding the buyer's
    /// signed maximum authorisation. Reserved by
    /// `schemes/upto/scheme_upto_evm.md` §4.
    /// Wire code: `invalid_upto_evm_payload_settlement_exceeds_amount`.
    InvalidUptoEvmPayloadSettlementExceedsAmount,
    /// `witness.facilitator` doesn't match any of the facilitator's signer
    /// addresses. Buyer signed a stale or unauthorised facilitator address.
    /// Wire code: `upto_facilitator_mismatch`.
    UptoFacilitatorMismatch,
    /// On-chain proxy reverted with `UnauthorizedFacilitator` (msg.sender
    /// is not the witness-bound facilitator).
    /// Wire code: `upto_unauthorized_facilitator`.
    UptoUnauthorizedFacilitator,
    /// On-chain proxy reverted with `AmountExceedsPermitted`.
    /// Wire code: `upto_amount_exceeds_permitted`.
    UptoAmountExceedsPermitted,

    // SVM exact scheme codes (chain-prefixed).
    /// SVM `extra.memo` field did not match the memo instruction data.
    /// Wire code: `invalid_exact_solana_payload_memo_mismatch`.
    InvalidExactSolanaPayloadMemoMismatch,
    /// Number of memo instructions attached is invalid (spec requires
    /// exactly one when `extra.memo` is declared).
    /// Wire code: `invalid_exact_solana_payload_memo_count`.
    InvalidExactSolanaPayloadMemoCount,

    // Catch-all preserving the wire code.
    /// Catch-all preserving any unknown wire code verbatim. Round-trips
    /// losslessly through serialisation so clients never lose information.
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
            Self::Custom(s) => s.as_str(),
        }
    }

    /// Constructs an [`ErrorReason`] from any wire code, mapping known
    /// strings to their canonical variants and preserving unknown codes
    /// as [`Self::Custom`].
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

/// A structured payment-problem record returned through wire boundaries.
///
/// Not itself serialisable — it is an internal, strongly-typed bridge between
/// Rust errors and [`crate::wire::VerifyResponse::Invalid`] /
/// [`crate::wire::SettleResponse::Failure`] on the wire.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaymentProblem {
    reason: ErrorReason,
    details: String,
}

impl PaymentProblem {
    /// Creates a new problem record.
    #[must_use]
    pub const fn new(reason: ErrorReason, details: String) -> Self {
        Self { reason, details }
    }

    /// Returns the wire-level reason code (clones the inner string for the
    /// `Custom` variant; cheap for unit variants).
    #[must_use]
    pub fn reason(&self) -> ErrorReason {
        self.reason.clone()
    }

    /// Returns a borrowed reference to the wire-level reason code.
    #[must_use]
    pub const fn reason_ref(&self) -> &ErrorReason {
        &self.reason
    }

    /// Returns the human-readable details.
    #[must_use]
    pub fn details(&self) -> &str {
        &self.details
    }
}

/// Trait for converting errors into [`PaymentProblem`]s.
pub trait AsPaymentProblem {
    /// Produces the canonical wire-level payment problem.
    fn as_payment_problem(&self) -> PaymentProblem;
}
