//! Error types for the EIP-155 exact scheme facilitator.

use alloy_primitives::TxHash;
use alloy_transport::TransportError;
use r402::facilitator::FacilitatorError;
use r402::proto::PaymentVerificationError;

use super::signature::StructuredSignatureFormatError;
use crate::chain::MetaTransactionSendError;

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
    /// Contract call failed.
    #[error("Contract call failed: {0}")]
    ContractCall(String),
    /// Payment verification failed.
    #[error(transparent)]
    PaymentVerification(#[from] PaymentVerificationError),
}

impl From<Eip155ExactError> for FacilitatorError {
    fn from(value: Eip155ExactError) -> Self {
        match value {
            Eip155ExactError::Transport(_)
            | Eip155ExactError::PendingTransaction(_)
            | Eip155ExactError::TransactionReverted(_)
            | Eip155ExactError::ContractCall(_) => Self::OnchainFailure(value.to_string()),
            Eip155ExactError::PaymentVerification(e) => Self::PaymentVerification(e),
        }
    }
}

impl From<StructuredSignatureFormatError> for Eip155ExactError {
    fn from(e: StructuredSignatureFormatError) -> Self {
        Self::PaymentVerification(PaymentVerificationError::InvalidSignature(e.to_string()))
    }
}

impl From<MetaTransactionSendError> for Eip155ExactError {
    fn from(e: MetaTransactionSendError) -> Self {
        match e {
            MetaTransactionSendError::Transport(e) => Self::Transport(e),
            MetaTransactionSendError::PendingTransaction(e) => Self::PendingTransaction(e),
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
            | alloy_provider::MulticallError::CallFailed(_) => Self::PaymentVerification(
                PaymentVerificationError::TransactionSimulation(e.to_string()),
            ),
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
