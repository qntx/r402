//! Error types for the Tron "exact" payment scheme.

use alloy_primitives::Address as EvmAddress;
use r402_protocol::error::VerificationError;

/// Errors specific to Tron exact scheme operations.
#[derive(Debug, thiserror::Error)]
pub enum TronExactError {
    /// The signature could not be recovered to a valid address.
    #[error("Can not recover signer from signature: {0}")]
    SignatureRecovery(String),
    /// The recovered signer does not match `authorization.from` / `permit2Authorization.from`.
    #[error("Signature does not match the declared sender")]
    SignerMismatch,
    /// The payer does not have sufficient token balance.
    #[error("Insufficient token balance")]
    InsufficientBalance,
    /// The authorization is not yet valid (`validAfter` is in the future).
    #[error("Authorization not yet valid")]
    NotYetValid,
    /// The authorization has expired (`validBefore` / `deadline` has passed).
    #[error("Authorization expired")]
    Expired,
    /// The authorization amount does not exactly match the required amount.
    #[error("Authorization value does not match required amount")]
    ValueMismatch,
    /// The authorization recipient does not match `payTo`.
    #[error("Authorization recipient does not match payment requirements")]
    RecipientMismatch,
    /// The authorization asset does not match the required asset.
    #[error("Authorization asset does not match payment requirements")]
    AssetMismatch,
    /// `permit2Authorization.spender` is not the canonical `x402ExactPermit2Proxy`.
    #[error("Permit2 spender is not the canonical x402ExactPermit2Proxy: {0}")]
    InvalidPermit2Spender(EvmAddress),
    /// The EIP-3009 authorization nonce has already been consumed on-chain.
    #[error("Authorization nonce already used")]
    NonceAlreadyUsed,
    /// The requested asset transfer method is not supported on this network.
    #[error("Asset transfer method not supported on this network")]
    UnsupportedTransferMethod,
    /// The chain reference in the payload does not match the configured provider.
    #[error("Chain mismatch")]
    ChainMismatch,
    /// A required TIP-712 domain parameter (`name`/`version`) is missing for EIP-3009.
    #[error("Missing TIP-712 domain parameters for EIP-3009 transfer")]
    MissingTip712Domain,
    /// The underlying `TronGrid` HTTP call failed.
    #[error("TronGrid request failed: {0}")]
    TronGrid(String),
    /// The broadcast transaction failed or was rejected by the network.
    #[error("Transaction failed: {0}")]
    TransactionFailed(String),
    /// Waiting for transaction confirmation timed out.
    #[error("Timed out waiting for transaction confirmation")]
    ConfirmationTimeout,
}

impl From<TronExactError> for VerificationError {
    fn from(e: TronExactError) -> Self {
        match e {
            TronExactError::SignatureRecovery(ref msg) => Self::InvalidSignature(msg.clone()),
            TronExactError::InsufficientBalance => Self::InsufficientFunds,
            TronExactError::NotYetValid => Self::Early,
            TronExactError::Expired => Self::Expired,
            TronExactError::ValueMismatch => Self::InvalidPaymentAmount,
            TronExactError::RecipientMismatch => Self::RecipientMismatch,
            TronExactError::AssetMismatch => Self::AssetMismatch,
            TronExactError::NonceAlreadyUsed => Self::NonceAlreadyUsed,
            TronExactError::ChainMismatch => Self::ChainIdMismatch,
            TronExactError::SignerMismatch
            | TronExactError::UnsupportedTransferMethod
            | TronExactError::MissingTip712Domain
            | TronExactError::InvalidPermit2Spender(_)
            | TronExactError::TransactionFailed(_)
            | TronExactError::ConfirmationTimeout
            | TronExactError::TronGrid(_) => Self::SimulationFailed(e.to_string()),
        }
    }
}

impl From<VerificationError> for TronExactError {
    fn from(e: VerificationError) -> Self {
        match e {
            VerificationError::InvalidSignature(msg) => Self::SignatureRecovery(msg),
            VerificationError::InsufficientFunds => Self::InsufficientBalance,
            VerificationError::Early => Self::NotYetValid,
            VerificationError::Expired => Self::Expired,
            VerificationError::InvalidPaymentAmount => Self::ValueMismatch,
            VerificationError::RecipientMismatch => Self::RecipientMismatch,
            VerificationError::AssetMismatch => Self::AssetMismatch,
            VerificationError::NonceAlreadyUsed => Self::NonceAlreadyUsed,
            VerificationError::ChainIdMismatch => Self::ChainMismatch,
            other => Self::TronGrid(other.to_string()),
        }
    }
}
