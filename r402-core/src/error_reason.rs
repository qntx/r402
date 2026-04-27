//! Machine-readable error codes used in x402 verify/settle responses.
//!
//! These codes form the canonical list shared across clients, servers, and
//! facilitators. When a new failure mode is introduced we add a variant here
//! first, then map chain-specific errors onto it via [`AsPaymentProblem`].

use std::fmt::{self, Display, Formatter};

use serde::{Deserialize, Serialize};

/// Canonical error code returned on the wire when a payment fails.
///
/// The enum is `#[non_exhaustive]`: new variants can appear without it being
/// a breaking change. Clients MUST handle unknown variants gracefully.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum ErrorReason {
    /// The payment payload format is invalid.
    InvalidFormat,
    /// The payment amount is incorrect (wrong, overpayment, or underpayment).
    InvalidPaymentAmount,
    /// The payment authorization is not yet valid (`validAfter` > now).
    InvalidPaymentEarly,
    /// The payment authorization has expired (`validBefore` ≤ now).
    InvalidPaymentExpired,
    /// The chain ID in the payload does not match the requirements.
    ChainIdMismatch,
    /// The recipient address does not match the requirements.
    RecipientMismatch,
    /// The token asset does not match the requirements.
    AssetMismatch,
    /// The accepted requirements do not match the declared requirements.
    AcceptedRequirementsMismatch,
    /// The cryptographic signature is invalid.
    InvalidSignature,
    /// On-chain transaction simulation failed before settlement.
    ///
    /// Distinct from [`Self::SimulationFailed`] (kept for backwards
    /// compatibility with older clients that spell the code differently).
    TransactionSimulation,
    /// Permit2 or EIP-3009 simulation failed before broadcast.
    SimulationFailed,
    /// Insufficient on-chain balance for the payment.
    InsufficientFunds,
    /// Permit2 allowance is insufficient; the payer must call `approve` on
    /// the token contract pointing at the Permit2 address first.
    ///
    /// x402 v2 HTTP transport maps this to HTTP `412 Precondition Failed`.
    Permit2AllowanceRequired,
    /// The chain is not supported by this facilitator.
    UnsupportedChain,
    /// The scheme is not supported by this facilitator.
    UnsupportedScheme,
    /// The EIP-3009 authorization nonce has already been consumed on-chain.
    NonceAlreadyUsed,
    /// The facilitator detected a duplicate settlement attempt and rejected it.
    DuplicateSettlement,
    /// The SVM `extra.memo` field did not match the memo instruction data.
    MemoMismatch,
    /// The number of memo instructions attached to a transaction is invalid
    /// (spec requires exactly one when `extra.memo` is declared).
    MemoInstructionCountInvalid,
    /// The resource server requested a settlement amount exceeding the
    /// buyer's signed maximum authorisation (upto scheme).
    ///
    /// Serialises as `invalid_upto_evm_payload_settlement_exceeds_amount`
    /// per the x402 v2 `schemes/upto/scheme_upto_evm.md` specification (§4).
    #[serde(rename = "invalid_upto_evm_payload_settlement_exceeds_amount")]
    SettlementExceedsAmount,
    /// `witness.facilitator` doesn't match any of the facilitator's signer
    /// addresses. Buyer signed a stale or unauthorised facilitator address.
    /// Mirrors the official `upto_facilitator_mismatch` constant.
    #[serde(rename = "upto_facilitator_mismatch")]
    UptoFacilitatorMismatch,
    /// On-chain proxy reverted with `UnauthorizedFacilitator` (msg.sender
    /// is not the witness-bound facilitator). Mirrors the official
    /// `upto_unauthorized_facilitator` constant.
    #[serde(rename = "upto_unauthorized_facilitator")]
    UptoUnauthorizedFacilitator,
    /// On-chain proxy reverted with `AmountExceedsPermitted`. Mirrors the
    /// official `upto_amount_exceeds_permitted` constant.
    #[serde(rename = "upto_amount_exceeds_permitted")]
    UptoAmountExceedsPermitted,
    /// A catch-all for unexpected errors (RPC failure, internal bug, ...).
    UnexpectedError,
}

impl ErrorReason {
    /// Returns the `snake_case` wire representation.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::InvalidFormat => "invalid_format",
            Self::InvalidPaymentAmount => "invalid_payment_amount",
            Self::InvalidPaymentEarly => "invalid_payment_early",
            Self::InvalidPaymentExpired => "invalid_payment_expired",
            Self::ChainIdMismatch => "chain_id_mismatch",
            Self::RecipientMismatch => "recipient_mismatch",
            Self::AssetMismatch => "asset_mismatch",
            Self::AcceptedRequirementsMismatch => "accepted_requirements_mismatch",
            Self::InvalidSignature => "invalid_signature",
            Self::TransactionSimulation => "transaction_simulation",
            Self::SimulationFailed => "simulation_failed",
            Self::InsufficientFunds => "insufficient_funds",
            Self::Permit2AllowanceRequired => "permit2_allowance_required",
            Self::UnsupportedChain => "unsupported_chain",
            Self::UnsupportedScheme => "unsupported_scheme",
            Self::NonceAlreadyUsed => "nonce_already_used",
            Self::DuplicateSettlement => "duplicate_settlement",
            Self::MemoMismatch => "memo_mismatch",
            Self::MemoInstructionCountInvalid => "memo_instruction_count_invalid",
            Self::SettlementExceedsAmount => "invalid_upto_evm_payload_settlement_exceeds_amount",
            Self::UptoFacilitatorMismatch => "upto_facilitator_mismatch",
            Self::UptoUnauthorizedFacilitator => "upto_unauthorized_facilitator",
            Self::UptoAmountExceedsPermitted => "upto_amount_exceeds_permitted",
            Self::UnexpectedError => "unexpected_error",
        }
    }
}

impl Display for ErrorReason {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
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

    /// Returns the wire-level reason code.
    #[must_use]
    pub const fn reason(&self) -> ErrorReason {
        self.reason
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
