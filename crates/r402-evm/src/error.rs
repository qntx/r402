//! Shared EVM facilitator errors.

use alloy_primitives::TxHash;
use alloy_transport::TransportError;
use r402_protocol::error::{FacilitatorError, VerificationError};

use crate::chain::MetaTransactionSendError;
use crate::signature::StructuredSignatureFormatError;

/// Errors specific to EIP-155 exact scheme operations.
#[derive(Debug, thiserror::Error)]
pub enum Eip155ExactError {
    /// RPC transport error.
    #[error(transparent)]
    Transport(#[from] TransportError),
    /// Pending transaction error.
    #[error(transparent)]
    PendingTransaction(#[from] alloy_provider::PendingTransactionError),
    /// On-chain transaction was reverted.
    #[error("Transaction {0} reverted")]
    TransactionReverted(TxHash),
    /// Receipt status was success but logs did not contain the expected ERC-20 Transfer.
    #[error("Transaction {0} missing matching Transfer event")]
    TransferEventMismatch(TxHash),
    /// Broadcast succeeded; waiting for the receipt failed.
    #[error("receipt wait failed for {hash}")]
    ReceiptWait {
        /// Broadcast transaction hash.
        hash: TxHash,
        /// Underlying receipt-wait error.
        #[source]
        source: alloy_provider::PendingTransactionError,
    },
    /// Contract call failed.
    #[error("Contract call failed: {0}")]
    ContractCall(String),
    /// Payment verification failed.
    #[error(transparent)]
    PaymentVerification(#[from] VerificationError),
}

impl From<Eip155ExactError> for FacilitatorError {
    fn from(value: Eip155ExactError) -> Self {
        match value {
            Eip155ExactError::Transport(_)
            | Eip155ExactError::PendingTransaction(_)
            | Eip155ExactError::TransactionReverted(_)
            | Eip155ExactError::TransferEventMismatch(_)
            | Eip155ExactError::ReceiptWait { .. }
            | Eip155ExactError::ContractCall(_) => Self::Onchain(value.to_string()),
            Eip155ExactError::PaymentVerification(e) => Self::Verification(e),
        }
    }
}

impl From<StructuredSignatureFormatError> for Eip155ExactError {
    fn from(e: StructuredSignatureFormatError) -> Self {
        Self::PaymentVerification(VerificationError::InvalidSignature(e.to_string()))
    }
}

impl From<MetaTransactionSendError> for Eip155ExactError {
    fn from(e: MetaTransactionSendError) -> Self {
        match e {
            MetaTransactionSendError::Transport(e) => Self::Transport(e),
            MetaTransactionSendError::PendingTransaction(e) => Self::PendingTransaction(e),
            MetaTransactionSendError::ReceiptWait { hash, source } => {
                Self::ReceiptWait { hash, source }
            }
            MetaTransactionSendError::Custom(e) => Self::ContractCall(e),
        }
    }
}

impl From<alloy_provider::MulticallError> for Eip155ExactError {
    fn from(e: alloy_provider::MulticallError) -> Self {
        match e {
            alloy_provider::MulticallError::ValueTx
            | alloy_provider::MulticallError::DecodeError(_)
            | alloy_provider::MulticallError::NoReturnData
            | alloy_provider::MulticallError::CallFailed(_) => {
                Self::PaymentVerification(VerificationError::SimulationFailed(e.to_string()))
            }
            alloy_provider::MulticallError::TransportError(transport_error) => {
                Self::Transport(transport_error)
            }
        }
    }
}

impl From<alloy_contract::Error> for Eip155ExactError {
    fn from(e: alloy_contract::Error) -> Self {
        match e {
            alloy_contract::Error::UnknownFunction(_)
            | alloy_contract::Error::UnknownSelector(_)
            | alloy_contract::Error::NotADeploymentTransaction
            | alloy_contract::Error::ContractNotDeployed
            | alloy_contract::Error::ZeroData(_, _)
            | alloy_contract::Error::AbiError(_) => Self::ContractCall(e.to_string()),
            alloy_contract::Error::TransportError(e) => Self::Transport(e),
            alloy_contract::Error::PendingTransactionError(e) => Self::PendingTransaction(e),
        }
    }
}
