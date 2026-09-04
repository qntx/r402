//! Layered error types for x402 facilitator operations.
//!
//! ```text
//! FacilitatorError
//! ├── Verification(VerificationError)
//! ├── Settlement(SettlementError)
//! ├── Aborted { reason, message }
//! ├── Onchain(String)
//! ├── Transport { kind: FacilitatorTransportKind }
//! └── Internal(Box<dyn Error>)
//! ```
//!
//! HTTP maps [`FacilitatorError::Transport`] to 502. Well-formed verify/settle
//! JSON on 4xx/5xx stays [`FacilitatorError::Verification`] /
//! [`FacilitatorError::Settlement`] (402).

mod problem;
mod reason;

pub use problem::{AsPaymentProblem, PaymentProblem};
pub use reason::ErrorReason;

/// Verification-phase failures.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum VerificationError {
    /// Payload JSON / binary could not be parsed.
    #[error("invalid format: {0}")]
    InvalidFormat(String),
    /// Payment amount is below the required amount.
    #[error("payment amount is below requirements")]
    InvalidPaymentAmount,
    /// Authorization is not yet valid.
    #[error("payment authorization is not yet valid")]
    Early,
    /// Authorization has expired.
    #[error("payment authorization has expired")]
    Expired,
    /// Chain ID does not match requirements.
    #[error("chain id mismatch")]
    ChainIdMismatch,
    /// Recipient does not match requirements.
    #[error("payment recipient mismatch")]
    RecipientMismatch,
    /// Token asset does not match requirements.
    #[error("payment asset mismatch")]
    AssetMismatch,
    /// On-chain balance is insufficient.
    #[error("insufficient on-chain balance")]
    InsufficientFunds,
    /// Permit2 allowance is insufficient. HTTP mapping: 412.
    #[error("permit2 allowance required")]
    Permit2AllowanceRequired,
    /// Signature is invalid.
    #[error("invalid signature: {0}")]
    InvalidSignature(String),
    /// Pre-settlement simulation failed.
    #[error("simulation failed: {0}")]
    SimulationFailed(String),
    /// Chain is not supported by this facilitator.
    #[error("unsupported chain")]
    UnsupportedChain,
    /// Scheme is not supported by this facilitator.
    #[error("unsupported scheme")]
    UnsupportedScheme,
    /// Accepted details diverge from declared requirements.
    #[error("accepted details do not match requirements")]
    AcceptedRequirementsMismatch,
    /// EIP-3009 nonce already consumed on-chain.
    #[error("authorization nonce already used")]
    NonceAlreadyUsed,
    /// Payload was already processed.
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
    /// Requested settlement exceeds the signed maximum (upto).
    #[error("settlement amount {requested} exceeds authorised maximum {authorised}")]
    SettlementAmountExceedsPermitted {
        /// Amount the resource server asked to settle.
        requested: String,
        /// Maximum amount signed by the buyer.
        authorised: String,
    },
    /// `witness.facilitator` is not an address known to this facilitator.
    #[error("witness.facilitator {witness} is not authorised on this facilitator")]
    UptoFacilitatorMismatch {
        /// EIP-55 checksummed `witness.facilitator` from the signed payload.
        witness: String,
    },
    /// On-chain proxy reverted with `UnauthorizedFacilitator`.
    #[error("on-chain proxy rejected settle: msg.sender does not match witness.facilitator")]
    UptoUnauthorizedFacilitator,
    /// On-chain proxy reverted with `AmountExceedsPermitted`.
    #[error("on-chain proxy rejected settle: amount exceeds permitted maximum")]
    UptoAmountExceedsPermitted,
    /// Official scheme wire reason not represented by a dedicated variant.
    #[error("{0}")]
    Wire(ErrorReason),
    /// Client extension echo does not match the server advertisement.
    #[error("extension_echo_mismatch")]
    ExtensionEchoMismatch {
        /// Extension key that failed echo validation.
        extension_key: String,
    },
}

impl VerificationError {
    /// Wraps an official scheme wire code (known variant or [`ErrorReason::Custom`]).
    #[must_use]
    pub fn from_wire(code: &str) -> Self {
        Self::Wire(ErrorReason::from_wire(code))
    }
}

impl From<serde_json::Error> for VerificationError {
    fn from(err: serde_json::Error) -> Self {
        Self::InvalidFormat(err.to_string())
    }
}

impl AsPaymentProblem for VerificationError {
    fn as_payment_problem(&self) -> PaymentProblem {
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
            Self::Wire(reason) => reason.clone(),
            Self::ExtensionEchoMismatch { .. } => ErrorReason::ExtensionEchoMismatch,
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

/// Facilitator transport failure (HTTP 502).
///
/// `HttpStatus` is a non-2xx response whose body is not well-formed
/// verify/settle JSON. `MalformedSuccessBody` is 2xx whose JSON is not a
/// [`crate::payment::VerifyResponse`] / [`crate::payment::SettleResponse`].
/// `Io` is connect/DNS/TLS/body-read (or auth-header resolution) before a
/// complete mapped HTTP response.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum FacilitatorTransportKind {
    /// Client-side wait elapsed before a response.
    #[error("facilitator request timed out")]
    Timeout,
    /// Non-2xx HTTP status without a well-formed verify/settle body.
    #[error("facilitator HTTP status {status}")]
    HttpStatus {
        /// HTTP status code.
        status: u16,
    },
    /// 2xx body is not a `VerifyResponse` / `SettleResponse`.
    #[error("facilitator returned a malformed success body")]
    MalformedSuccessBody,
    /// Connect, DNS, TLS, body-read, or auth-header resolution failed.
    #[error("facilitator I/O failure")]
    Io,
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
    /// Hook veto before or during the operation.
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
    /// Transport failure. HTTP mapping: 502.
    #[error("{kind}")]
    Transport {
        /// Transport failure kind.
        kind: FacilitatorTransportKind,
    },
    /// Unexpected internal error.
    #[error(transparent)]
    Internal(Box<dyn std::error::Error + Send + Sync>),
}

impl FacilitatorError {
    /// Constructs an aborted variant.
    #[must_use]
    pub fn aborted(reason: impl Into<String>, message: impl Into<String>) -> Self {
        Self::Aborted {
            reason: reason.into(),
            message: message.into(),
        }
    }

    /// Constructs a transport failure.
    #[must_use]
    pub const fn transport(kind: FacilitatorTransportKind) -> Self {
        Self::Transport { kind }
    }

    /// Returns `true` when this is a transport failure.
    #[must_use]
    pub const fn is_transport(&self) -> bool {
        matches!(self, Self::Transport { .. })
    }

    /// Constructs an internal error from any boxed error.
    #[must_use]
    pub fn internal<E>(err: E) -> Self
    where
        E: Into<Box<dyn std::error::Error + Send + Sync>>,
    {
        Self::Internal(err.into())
    }

    /// 402 payment problem. `None` for [`Self::Transport`] (HTTP 502, not 402).
    #[must_use]
    pub fn as_payment_problem(&self) -> Option<PaymentProblem> {
        match self {
            Self::Verification(e) => Some(e.as_payment_problem()),
            Self::Settlement(e) => Some(e.as_payment_problem()),
            Self::Aborted { reason, message } => Some(PaymentProblem::new(
                ErrorReason::from_wire(reason),
                format!("{reason}: {message}"),
            )),
            Self::Onchain(message) => Some(PaymentProblem::new(
                ErrorReason::InvalidTransactionState,
                message.clone(),
            )),
            Self::Transport { .. } => None,
            Self::Internal(e) => Some(PaymentProblem::new(
                ErrorReason::UnexpectedVerifyError,
                e.to_string(),
            )),
        }
    }
}

impl From<FacilitatorTransportKind> for FacilitatorError {
    fn from(kind: FacilitatorTransportKind) -> Self {
        Self::Transport { kind }
    }
}

/// Client-side error returned when constructing or signing payments.
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
    /// Spend controls rejected every remaining payment option.
    #[error("{0}")]
    SpendControls(String),
    /// No remaining accept has a recognized `extra.paymentFlow`.
    #[error("no payment requirements with a recognized paymentFlow")]
    UnrecognizedPaymentFlow,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transport_is_distinct_from_verification() {
        let err = FacilitatorError::transport(FacilitatorTransportKind::Timeout);
        assert!(err.is_transport());
        assert!(!matches!(err, FacilitatorError::Verification(_)));
        assert!(
            err.as_payment_problem().is_none(),
            "transport is HTTP 502, not a 402 payment problem"
        );
    }

    #[test]
    fn http_status_kind_preserves_code() {
        let kind = FacilitatorTransportKind::HttpStatus { status: 503 };
        assert_eq!(kind.to_string(), "facilitator HTTP status 503");
        let err = FacilitatorError::from(kind);
        match err {
            FacilitatorError::Transport {
                kind: FacilitatorTransportKind::HttpStatus { status },
            } => assert_eq!(status, 503, "status must round-trip"),
            other => panic!("expected transport, got {other:?}"),
        }
    }

    #[test]
    fn io_kind_is_transport_not_payment_problem() {
        let err = FacilitatorError::transport(FacilitatorTransportKind::Io);
        assert!(err.is_transport());
        assert!(
            err.as_payment_problem().is_none(),
            "I/O transport is HTTP 502, not a 402 payment problem"
        );
    }

    #[test]
    fn from_wire_preserves_official_exact_codes() {
        let err = VerificationError::from_wire("eip6492_factory_not_allowed");
        assert_eq!(
            err.as_payment_problem().reason().as_str(),
            "eip6492_factory_not_allowed"
        );
        let deployed = VerificationError::from_wire("asset_not_deployed_contract");
        assert_eq!(
            deployed.as_payment_problem().reason().as_str(),
            "asset_not_deployed_contract"
        );
        let mismatch = VerificationError::from_wire("invalid_exact_evm_transfer_event_mismatch");
        assert_eq!(
            mismatch.as_payment_problem().reason().as_str(),
            "invalid_exact_evm_transfer_event_mismatch"
        );
    }

    #[test]
    fn extension_echo_mismatch_maps_to_wire_reason() {
        let err = VerificationError::ExtensionEchoMismatch {
            extension_key: "builder-code".into(),
        };
        let problem = err.as_payment_problem();
        assert_eq!(problem.reason(), ErrorReason::ExtensionEchoMismatch);
        let wrapped = FacilitatorError::from(err);
        let Some(wrapped_problem) = wrapped.as_payment_problem() else {
            panic!("verification errors must map to a 402 payment problem");
        };
        assert_eq!(wrapped_problem.reason(), ErrorReason::ExtensionEchoMismatch);
    }
}
