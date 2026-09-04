//! Sign-In with X (`sign-in-with-x`) extension.
//!
//! Challenge advertisement, origin binding, paid-address/nonce storage, and
//! CAIP-122 verification. HTTP `GrantAccess` lives in `r402-http`.
//! Client proof is the `SIGN-IN-WITH-X` header, not `PaymentPayload.extensions`.

pub mod advertise;
pub mod client;
pub mod origin;
pub mod paid;
pub mod proof;

pub use advertise::{SiwxChain, SiwxExtension};
pub use client::{SiwxClientExtension, SiwxSigner};
pub use origin::{SiwxOrigin, SiwxOriginError};
pub use paid::{InMemoryPaidAddressStore, PaidAddressStore};
pub use proof::{SiwxProof, SiwxProofError};
pub use siwx_evm::EvmVerifier;

/// Wire key for the Sign-In with X extension.
pub const SIWX_KEY: &str = "sign-in-with-x";

/// HTTP request header carrying a SIWX proof. Never emit as a response header.
pub const SIWX_HTTP_HEADER: &str = "SIGN-IN-WITH-X";

/// Default `issuedAt` age accepted during field validation.
pub const DEFAULT_MAX_ISSUED_AGE: time::Duration = time::Duration::minutes(5);

/// Default challenge `expirationTime` offset from `issuedAt`.
pub const DEFAULT_CHALLENGE_TTL: time::Duration = time::Duration::minutes(5);

/// Spec `invalid_siwx_*` failure codes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum SiwxError {
    /// `domain` does not match the configured public origin host.
    #[error("invalid_siwx_domain_mismatch")]
    DomainMismatch,
    /// `uri` origin does not match the configured public origin.
    #[error("invalid_siwx_uri_mismatch")]
    UriMismatch,
    /// `issuedAt` is not a valid timestamp.
    #[error("invalid_siwx_issued_at")]
    IssuedAt,
    /// `issuedAt` exceeds the maximum age.
    #[error("invalid_siwx_issued_at_too_old")]
    IssuedAtTooOld,
    /// `issuedAt` is in the future.
    #[error("invalid_siwx_issued_at_in_future")]
    IssuedAtInFuture,
    /// `expirationTime` is not a valid timestamp.
    #[error("invalid_siwx_expiration_time")]
    ExpirationTime,
    /// `expirationTime` is in the past.
    #[error("invalid_siwx_expired")]
    Expired,
    /// `notBefore` is not a valid timestamp.
    #[error("invalid_siwx_not_before")]
    NotBefore,
    /// `notBefore` is in the future.
    #[error("invalid_siwx_not_yet_valid")]
    NotYetValid,
    /// Nonce failed uniqueness or format validation.
    #[error("invalid_siwx_nonce")]
    Nonce,
    /// Cryptographic signature verification failed.
    #[error("invalid_siwx_signature")]
    Signature,
    /// `chainId` is not a valid CAIP-2 identifier.
    #[error("invalid_siwx_chain_id")]
    ChainId,
    /// `chainId` namespace is not supported.
    #[error("invalid_siwx_unsupported_chain")]
    UnsupportedChain,
    /// Signature or address encoding is invalid.
    #[error("invalid_siwx_malformed_signature")]
    MalformedSignature,
    /// Verifier threw (for example RPC unavailable during EIP-1271).
    #[error("invalid_siwx_verifier_error")]
    VerifierError,
}

impl SiwxError {
    /// Spec machine-readable code.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::DomainMismatch => "invalid_siwx_domain_mismatch",
            Self::UriMismatch => "invalid_siwx_uri_mismatch",
            Self::IssuedAt => "invalid_siwx_issued_at",
            Self::IssuedAtTooOld => "invalid_siwx_issued_at_too_old",
            Self::IssuedAtInFuture => "invalid_siwx_issued_at_in_future",
            Self::ExpirationTime => "invalid_siwx_expiration_time",
            Self::Expired => "invalid_siwx_expired",
            Self::NotBefore => "invalid_siwx_not_before",
            Self::NotYetValid => "invalid_siwx_not_yet_valid",
            Self::Nonce => "invalid_siwx_nonce",
            Self::Signature => "invalid_siwx_signature",
            Self::ChainId => "invalid_siwx_chain_id",
            Self::UnsupportedChain => "invalid_siwx_unsupported_chain",
            Self::MalformedSignature => "invalid_siwx_malformed_signature",
            Self::VerifierError => "invalid_siwx_verifier_error",
        }
    }
}
