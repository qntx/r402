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

use crate::error_reason::{AsPaymentProblem, ErrorReason, PaymentProblem};

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

    /// The payment amount does not match the requirements.
    #[error("payment amount does not match requirements")]
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
    /// Maps to `ErrorReason::SettlementExceedsAmount` on the wire, which
    /// serialises as `invalid_upto_evm_payload_settlement_exceeds_amount`
    /// per x402 v2 spec.
    #[error("settlement amount {requested} exceeds authorised maximum {authorised}")]
    SettlementAmountExceedsPermitted {
        /// Amount the resource server asked to settle.
        requested: String,
        /// Maximum amount signed by the buyer.
        authorised: String,
    },
}

impl From<serde_json::Error> for VerificationError {
    fn from(err: serde_json::Error) -> Self {
        Self::InvalidFormat(err.to_string())
    }
}

impl AsPaymentProblem for VerificationError {
    fn as_payment_problem(&self) -> PaymentProblem {
        let reason = match self {
            Self::InvalidFormat(_) => ErrorReason::InvalidFormat,
            Self::InvalidPaymentAmount => ErrorReason::InvalidPaymentAmount,
            Self::Early => ErrorReason::InvalidPaymentEarly,
            Self::Expired => ErrorReason::InvalidPaymentExpired,
            Self::ChainIdMismatch => ErrorReason::ChainIdMismatch,
            Self::RecipientMismatch => ErrorReason::RecipientMismatch,
            Self::AssetMismatch => ErrorReason::AssetMismatch,
            Self::InsufficientFunds => ErrorReason::InsufficientFunds,
            Self::Permit2AllowanceRequired => ErrorReason::Permit2AllowanceRequired,
            Self::InvalidSignature(_) => ErrorReason::InvalidSignature,
            Self::SimulationFailed(_) => ErrorReason::SimulationFailed,
            Self::UnsupportedChain => ErrorReason::UnsupportedChain,
            Self::UnsupportedScheme => ErrorReason::UnsupportedScheme,
            Self::AcceptedRequirementsMismatch => ErrorReason::AcceptedRequirementsMismatch,
            Self::NonceAlreadyUsed => ErrorReason::NonceAlreadyUsed,
            Self::DuplicateSettlement => ErrorReason::DuplicateSettlement,
            Self::MemoMismatch => ErrorReason::MemoMismatch,
            Self::MemoInstructionCountInvalid { .. } => ErrorReason::MemoInstructionCountInvalid,
            Self::SettlementAmountExceedsPermitted { .. } => ErrorReason::SettlementExceedsAmount,
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
            Self::Onchain(_) | Self::Timeout => ErrorReason::UnexpectedError,
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
            Self::Aborted { reason, message } => {
                PaymentProblem::new(ErrorReason::UnexpectedError, format!("{reason}: {message}"))
            }
            Self::Onchain(message) => {
                PaymentProblem::new(ErrorReason::UnexpectedError, message.clone())
            }
            Self::Internal(e) => PaymentProblem::new(ErrorReason::UnexpectedError, e.to_string()),
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
