//! Error types for the Casper exact scheme.

use r402_protocol::error::VerificationError;

use crate::chain::CasperChainReferenceFormatError;
use crate::chain::motes::MotesParseError;

/// Errors specific to Casper exact scheme operations.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum CasperExactError {
    /// The declared scheme was not `exact`.
    #[error("unsupported scheme, expected exact")]
    InvalidScheme,
    /// `paymentPayload.accepted.network` and `paymentRequirements.network`
    /// disagree.
    #[error("network mismatch: payload={payload} requirements={requirements}")]
    NetworkMismatch {
        /// Network declared by the buyer's payload.
        payload: String,
        /// Network declared by the seller's requirements.
        requirements: String,
    },
    /// `paymentPayload.accepted` does not satisfy `paymentRequirements`.
    #[error("accepted requirements do not match payment requirements")]
    AcceptedRequirementsMismatch,
    /// The network is not a supported Casper chain.
    #[error("unsupported casper network: {0}")]
    UnsupportedNetwork(#[from] CasperChainReferenceFormatError),
    /// `requirements.asset` is not a 64-character CEP-18 package hash.
    #[error("invalid asset (expected 64-char contract package hash): {0}")]
    InvalidAsset(String),
    /// `requirements.payTo` is not a valid Casper addressable key.
    #[error("invalid payTo address: {0}")]
    InvalidPayTo(String),
    /// `authorization.from` is not a valid Casper addressable key.
    #[error("invalid payer address: {0}")]
    InvalidPayer(String),
    /// `authorization.to` does not equal `requirements.payTo`.
    #[error("payTo mismatch: authorization.to={authorization} requirements.payTo={requirements}")]
    PayToMismatch {
        /// Recipient the buyer signed for.
        authorization: String,
        /// Recipient the seller requires.
        requirements: String,
    },
    /// `authorization.value` does not equal `requirements.amount`.
    #[error(
        "amount mismatch: authorization.value={authorization} requirements.amount={requirements}"
    )]
    AmountMismatch {
        /// Amount the buyer signed for.
        authorization: String,
        /// Amount the seller requires.
        requirements: String,
    },
    /// The payment amount was zero.
    #[error("payment amount must be non-zero")]
    ZeroAmount,
    /// The amount could not be converted to base units.
    #[error("invalid amount: {0}")]
    InvalidAmount(#[from] MotesParseError),
    /// `validAfter` is in the future.
    #[error("authorization is not yet valid: validAfter={valid_after} now={now}")]
    NotYetValid {
        /// Signed `validAfter` timestamp.
        valid_after: u64,
        /// Current wall-clock time.
        now: u64,
    },
    /// `validBefore` has passed, or is too close to leave time to settle.
    #[error("authorization expired or too close to expiry: validBefore={valid_before} now={now}")]
    Expired {
        /// Signed `validBefore` timestamp.
        valid_before: u64,
        /// Current wall-clock time.
        now: u64,
    },
    /// The signature was malformed (wrong length or not hex).
    #[error("invalid signature: {0}")]
    InvalidSignature(String),
    /// The nonce was malformed (wrong length or not hex).
    #[error("invalid nonce: {0}")]
    InvalidNonce(String),
    /// The payer public key does not correspond to `authorization.from`.
    #[error("public key does not match authorization.from")]
    PublicKeyMismatch,
    /// `extra.name` was missing or empty.
    #[error("requirements.extra.name is required for the CEP-18 EIP-712 domain")]
    MissingTokenName,
    /// `extra.version` was missing or empty.
    #[error("requirements.extra.version is required for the CEP-18 EIP-712 domain")]
    MissingTokenVersion,
}

impl From<CasperExactError> for VerificationError {
    fn from(error: CasperExactError) -> Self {
        match error {
            CasperExactError::InvalidScheme => Self::UnsupportedScheme,
            CasperExactError::NetworkMismatch { .. } => Self::ChainIdMismatch,
            CasperExactError::AcceptedRequirementsMismatch => Self::AcceptedRequirementsMismatch,
            CasperExactError::UnsupportedNetwork(_) => Self::UnsupportedChain,
            CasperExactError::InvalidAsset(_) => Self::AssetMismatch,
            CasperExactError::PayToMismatch { .. } => Self::RecipientMismatch,
            CasperExactError::AmountMismatch { .. } | CasperExactError::ZeroAmount => {
                Self::InvalidPaymentAmount
            }
            CasperExactError::NotYetValid { .. } => Self::Early,
            CasperExactError::Expired { .. } => Self::Expired,
            CasperExactError::InvalidSignature(_) | CasperExactError::PublicKeyMismatch => {
                Self::InvalidSignature(error.to_string())
            }
            CasperExactError::InvalidPayTo(_)
            | CasperExactError::InvalidPayer(_)
            | CasperExactError::InvalidNonce(_)
            | CasperExactError::InvalidAmount(_)
            | CasperExactError::MissingTokenName
            | CasperExactError::MissingTokenVersion => Self::InvalidFormat(error.to_string()),
        }
    }
}

#[cfg(test)]
mod tests {
    use r402_protocol::error::{AsPaymentProblem, ErrorReason};

    use super::*;

    fn reason(error: CasperExactError) -> ErrorReason {
        VerificationError::from(error).as_payment_problem().reason()
    }

    #[test]
    fn scheme_and_network_errors_map_to_spec_reasons() {
        assert_eq!(
            reason(CasperExactError::InvalidScheme),
            ErrorReason::UnsupportedScheme
        );
        assert_eq!(
            reason(CasperExactError::NetworkMismatch {
                payload: "casper:casper".to_owned(),
                requirements: "casper:casper-test".to_owned(),
            }),
            ErrorReason::InvalidNetwork
        );
        assert_eq!(
            reason(CasperExactError::AcceptedRequirementsMismatch),
            ErrorReason::InvalidPaymentRequirements
        );
    }

    #[test]
    fn amount_errors_map_to_payment_requirements() {
        assert_eq!(
            reason(CasperExactError::ZeroAmount),
            ErrorReason::InvalidPaymentRequirements
        );
        assert_eq!(
            reason(CasperExactError::AmountMismatch {
                authorization: "1".to_owned(),
                requirements: "2".to_owned(),
            }),
            ErrorReason::InvalidPaymentRequirements
        );
    }

    #[test]
    fn timing_and_signature_errors_map_to_invalid_payload() {
        assert_eq!(
            reason(CasperExactError::Expired {
                valid_before: 1,
                now: 2
            }),
            ErrorReason::InvalidPayload
        );
        assert_eq!(
            reason(CasperExactError::NotYetValid {
                valid_after: 2,
                now: 1
            }),
            ErrorReason::InvalidPayload
        );
        assert_eq!(
            reason(CasperExactError::PublicKeyMismatch),
            ErrorReason::InvalidPayload
        );
    }

    #[test]
    fn missing_domain_fields_map_to_invalid_payload() {
        assert_eq!(
            reason(CasperExactError::MissingTokenName),
            ErrorReason::InvalidPayload
        );
        assert_eq!(
            reason(CasperExactError::MissingTokenVersion),
            ErrorReason::InvalidPayload
        );
    }

    #[test]
    fn sub_mote_precision_surfaces_as_invalid_format() {
        let error = CasperExactError::from(MotesParseError::SubMotePrecision { digits: 10 });
        let verification = VerificationError::from(error);
        assert!(
            verification.to_string().contains("fractional digits"),
            "precision failures must not be silently swallowed: {verification}"
        );
    }
}
