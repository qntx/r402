//! Layered error types for x402 facilitator operations.
//!
//! The hierarchy is:
//!
//! ```text
//! FacilitatorError
//! ├── Verification(VerificationError)
//! ├── Settlement(SettlementError)
//! ├── Aborted { reason, message }   // hook-level veto
//! ├── Onchain(String)               // chain RPC / tx revert
//! └── Internal(Box<dyn Error>)      // unexpected internal failure
//! ```
//!
//! Every leaf carries enough information to round-trip through the wire via
//! [`AsPaymentProblem`], which chain-specific crates implement for their own
//! error variants.
//!
//! [`ErrorReason`] is the machine-readable wire code (x402 v2 spec §9).

use std::fmt::{self, Display, Formatter};

use compact_str::CompactString;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// Verification-phase failures.
///
/// Returned when payload parsing, signature verification, or pre-settlement
/// on-chain checks reject the request.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum VerificationError {
    /// The payload JSON / binary format could not be parsed.
    #[error("invalid format: {0}")]
    InvalidFormat(String),

    /// The payment amount is below the required amount.
    ///
    /// For SVM exact, overpayment (`actual >= required`) is allowed per
    /// `scheme_exact_svm.md` §1.4; only underpayment maps here.
    #[error("payment amount is below requirements")]
    InvalidPaymentAmount,

    /// The payment authorization is not yet valid.
    #[error("payment authorization is not yet valid")]
    Early,

    /// The payment authorization has expired.
    #[error("payment authorization has expired")]
    Expired,

    /// The chain ID does not match requirements.
    #[error("chain id mismatch")]
    ChainIdMismatch,

    /// The payment recipient does not match requirements.
    #[error("payment recipient mismatch")]
    RecipientMismatch,

    /// The token asset does not match requirements.
    #[error("payment asset mismatch")]
    AssetMismatch,

    /// On-chain balance is insufficient.
    #[error("insufficient on-chain balance")]
    InsufficientFunds,

    /// Permit2 allowance is insufficient.
    ///
    /// Clients receiving this should call `approve` on the token contract
    /// pointing at the Permit2 address and retry. HTTP transports map this
    /// to `412 Precondition Failed`.
    #[error("permit2 allowance required")]
    Permit2AllowanceRequired,

    /// The payment signature is invalid.
    #[error("invalid signature: {0}")]
    InvalidSignature(String),

    /// Pre-settlement simulation failed.
    #[error("simulation failed: {0}")]
    SimulationFailed(String),

    /// The chain is not supported by this facilitator.
    #[error("unsupported chain")]
    UnsupportedChain,

    /// The scheme is not supported by this facilitator.
    #[error("unsupported scheme")]
    UnsupportedScheme,

    /// Accepted details diverge from declared requirements.
    #[error("accepted details do not match requirements")]
    AcceptedRequirementsMismatch,

    /// EIP-3009 nonce already consumed on-chain.
    #[error("authorization nonce already used")]
    NonceAlreadyUsed,

    /// Attempted to settle a transaction whose payload was already processed.
    #[error("duplicate settlement attempt")]
    DuplicateSettlement,

    /// Memo data does not match `extra.memo`.
    #[error("memo data mismatch")]
    MemoMismatch,

    /// Invalid number of memo instructions (spec requires exactly one).
    #[error("memo instruction count invalid (expected 1, got {count})")]
    MemoInstructionCountInvalid {
        /// Observed number of memo instructions.
        count: usize,
    },

    /// Resource server requested a settlement amount exceeding the signed
    /// maximum authorisation (upto scheme).
    ///
    /// Maps to `ErrorReason::InvalidUptoEvmPayloadSettlementExceedsAmount`
    /// on the wire, which serialises as
    /// `invalid_upto_evm_payload_settlement_exceeds_amount` per x402 v2 spec.
    #[error("settlement amount {requested} exceeds authorised maximum {authorised}")]
    SettlementAmountExceedsPermitted {
        /// Amount the resource server asked to settle.
        requested: String,
        /// Maximum amount signed by the buyer.
        authorised: String,
    },

    /// `witness.facilitator` does not match any address known to the
    /// facilitator (upto scheme).
    ///
    /// Buyer either signed with a stale `paymentRequirements.extra.facilitatorAddress`
    /// or attempted to bind the payment to a different facilitator instance.
    /// Maps to `ErrorReason::UptoFacilitatorMismatch`.
    #[error("witness.facilitator {witness} is not authorised on this facilitator")]
    UptoFacilitatorMismatch {
        /// EIP-55 checksummed `witness.facilitator` from the buyer's signed payload.
        witness: String,
    },

    /// On-chain proxy reverted with `UnauthorizedFacilitator` because the
    /// settle transaction was submitted from an address other than
    /// `witness.facilitator` (upto scheme).
    ///
    /// Maps to `ErrorReason::UptoUnauthorizedFacilitator`.
    #[error("on-chain proxy rejected settle: msg.sender does not match witness.facilitator")]
    UptoUnauthorizedFacilitator,

    /// On-chain proxy reverted with `AmountExceedsPermitted` (upto scheme).
    /// Defence-in-depth signal: the off-chain pipeline should have caught
    /// this before submission.
    ///
    /// Maps to `ErrorReason::UptoAmountExceedsPermitted`.
    #[error("on-chain proxy rejected settle: amount exceeds permitted maximum")]
    UptoAmountExceedsPermitted,
}

impl From<serde_json::Error> for VerificationError {
    fn from(err: serde_json::Error) -> Self {
        Self::InvalidFormat(err.to_string())
    }
}

impl AsPaymentProblem for VerificationError {
    fn as_payment_problem(&self) -> PaymentProblem {
        // Chain-agnostic mappings target the spec §9 standard codes.
        // Chain-specific facilitators may override with chain-prefixed
        // codes (e.g. `invalid_exact_evm_payload_signature`) at their own
        // `AsPaymentProblem` impls.
        let reason = match self {
            Self::InvalidFormat(_) | Self::InvalidSignature(_) | Self::Early | Self::Expired => {
                ErrorReason::InvalidPayload
            }
            Self::InvalidPaymentAmount
            | Self::RecipientMismatch
            | Self::AssetMismatch
            | Self::AcceptedRequirementsMismatch => ErrorReason::InvalidPaymentRequirements,
            Self::ChainIdMismatch | Self::UnsupportedChain => ErrorReason::InvalidNetwork,
            Self::InsufficientFunds => ErrorReason::InsufficientFunds,
            Self::Permit2AllowanceRequired => ErrorReason::Permit2AllowanceRequired,
            Self::SimulationFailed(_) => ErrorReason::InvalidTransactionState,
            Self::UnsupportedScheme => ErrorReason::UnsupportedScheme,
            Self::NonceAlreadyUsed => ErrorReason::NonceAlreadyUsed,
            Self::DuplicateSettlement => ErrorReason::DuplicateSettlement,
            Self::MemoMismatch => ErrorReason::InvalidExactSolanaPayloadMemoMismatch,
            Self::MemoInstructionCountInvalid { .. } => {
                ErrorReason::InvalidExactSolanaPayloadMemoCount
            }
            Self::SettlementAmountExceedsPermitted { .. } => {
                ErrorReason::InvalidUptoEvmPayloadSettlementExceedsAmount
            }
            Self::UptoFacilitatorMismatch { .. } => ErrorReason::UptoFacilitatorMismatch,
            Self::UptoUnauthorizedFacilitator => ErrorReason::UptoUnauthorizedFacilitator,
            Self::UptoAmountExceedsPermitted => ErrorReason::UptoAmountExceedsPermitted,
        };
        PaymentProblem::new(reason, self.to_string())
    }
}

/// Settlement-phase failures.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum SettlementError {
    /// Settlement reverted on-chain or the RPC call failed.
    #[error("on-chain settlement failed: {0}")]
    Onchain(String),

    /// Settlement timed out.
    #[error("settlement timed out")]
    Timeout,

    /// Duplicate settlement detected at the cache layer.
    #[error("duplicate settlement attempt")]
    Duplicate,
}

impl AsPaymentProblem for SettlementError {
    fn as_payment_problem(&self) -> PaymentProblem {
        let reason = match self {
            Self::Onchain(_) => ErrorReason::InvalidTransactionState,
            Self::Timeout => ErrorReason::UnexpectedSettleError,
            Self::Duplicate => ErrorReason::DuplicateSettlement,
        };
        PaymentProblem::new(reason, self.to_string())
    }
}

/// Top-level facilitator failure.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum FacilitatorError {
    /// Verification-phase failure.
    #[error(transparent)]
    Verification(#[from] VerificationError),

    /// Settlement-phase failure.
    #[error(transparent)]
    Settlement(#[from] SettlementError),

    /// Hook veto: the operation was aborted before or during its lifecycle.
    #[error("{reason}: {message}")]
    Aborted {
        /// Machine-readable abort reason (e.g. `"kyt_blocked"`).
        reason: String,
        /// Human-readable abort description.
        message: String,
    },

    /// On-chain or RPC failure that is not scheme-specific.
    #[error("on-chain error: {0}")]
    Onchain(String),

    /// Any other internal error not covered above.
    #[error(transparent)]
    Internal(Box<dyn std::error::Error + Send + Sync>),
}

impl FacilitatorError {
    /// Constructs an aborted variant from reason and message strings.
    #[must_use]
    pub fn aborted(reason: impl Into<String>, message: impl Into<String>) -> Self {
        Self::Aborted {
            reason: reason.into(),
            message: message.into(),
        }
    }

    /// Constructs an internal error from any boxed error.
    #[must_use]
    pub fn internal<E>(err: E) -> Self
    where
        E: Into<Box<dyn std::error::Error + Send + Sync>>,
    {
        Self::Internal(err.into())
    }
}

impl AsPaymentProblem for FacilitatorError {
    fn as_payment_problem(&self) -> PaymentProblem {
        match self {
            Self::Verification(e) => e.as_payment_problem(),
            Self::Settlement(e) => e.as_payment_problem(),
            // Hook-supplied reason strings round-trip through `from_wire`,
            // which preserves any unknown code as `Custom(…)`.
            Self::Aborted { reason, message } => PaymentProblem::new(
                ErrorReason::from_wire(reason),
                format!("{reason}: {message}"),
            ),
            Self::Onchain(message) => {
                PaymentProblem::new(ErrorReason::InvalidTransactionState, message.clone())
            }
            Self::Internal(e) => {
                PaymentProblem::new(ErrorReason::UnexpectedVerifyError, e.to_string())
            }
        }
    }
}

/// Client-side error returned when constructing / signing payments.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ClientError {
    /// No candidate matched the client's capabilities.
    #[error("no matching payment option")]
    NoMatchingPaymentOption,

    /// The underlying HTTP body cannot be retried (streaming, etc.).
    #[error("request is not cloneable (streaming body?)")]
    RequestNotCloneable,

    /// Parsing the 402 response body failed.
    #[error("failed to parse 402 response: {0}")]
    Parse(String),

    /// Signing the authorization failed.
    #[error("failed to sign payment: {0}")]
    Signing(String),

    /// A required on-chain pre-condition is missing (e.g. approval).
    #[error("payment pre-condition not met: {0}")]
    PreConditionFailed(String),

    /// JSON (de)serialisation failed.
    #[error(transparent)]
    Json(#[from] serde_json::Error),
}

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
    /// Settlement was broadcast but confirmation could not be established.
    /// Wire code: `settlement_pending`.
    ///
    /// Non-terminal: the transaction may still confirm. A
    /// [`crate::wire::SettleResponse::Failure`] with this reason MUST carry a
    /// non-empty `transaction` (spec §9). Emit only via
    /// [`crate::PendingSettlementStore`].
    SettlementPending,

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

#[cfg(test)]
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
}
