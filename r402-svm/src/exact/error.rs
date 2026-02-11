//! Error types for the Solana "exact" payment scheme.
//!
//! This module centralizes all error types used across the exact scheme's
//! client, server, and facilitator components.

use r402::proto::PaymentVerificationError;
use solana_pubkey::Pubkey;

/// Errors specific to Solana exact scheme operations.
#[derive(Debug, thiserror::Error)]
pub enum SolanaExactError {
    /// Transaction could not be deserialized.
    #[error("Can not decode transaction: {0}")]
    TransactionDecoding(String),
    /// Compute unit limit exceeds facilitator maximum.
    #[error("Compute unit limit exceeds facilitator maximum")]
    MaxComputeUnitLimitExceeded,
    /// Compute unit price exceeds facilitator maximum.
    #[error("Compute unit price exceeds facilitator maximum")]
    MaxComputeUnitPriceExceeded,
    /// Transaction has too few instructions.
    #[error("Too few instructions in transaction")]
    TooFewInstructions,
    /// Additional instructions are not permitted.
    #[error("Additional instructions not allowed")]
    AdditionalInstructionsNotAllowed,
    /// Instruction count exceeds the maximum allowed.
    #[error("Instruction count exceeds maximum: {0}")]
    InstructionCountExceedsMax(usize),
    /// Transaction contains a blocked program.
    #[error("Blocked program in transaction: {0}")]
    BlockedProgram(Pubkey),
    /// Program is not in the allowed list.
    #[error("Program not in allowed list: {0}")]
    ProgramNotAllowed(Pubkey),
    /// ATA creation instruction is not supported.
    #[error("CreateATA instruction not supported - destination ATA must exist")]
    CreateATANotSupported,
    /// Fee payer was found in instruction accounts.
    #[error("Fee payer included in instruction accounts")]
    FeePayerIncludedInInstructionAccounts,
    /// Fee payer is transferring funds, which is not allowed.
    #[error("Fee payer found transferring funds")]
    FeePayerTransferringFunds,
    /// No instruction found at the given index.
    #[error("Instruction at index {0} not found")]
    NoInstructionAtIndex(usize),
    /// No account found at the given index.
    #[error("No account at index {0}")]
    NoAccountAtIndex(u8),
    /// Instruction at the given index has no data or accounts.
    #[error("Empty instruction at index {0}")]
    EmptyInstructionAtIndex(usize),
    /// Compute limit instruction could not be parsed.
    #[error("Invalid compute limit instruction")]
    InvalidComputeLimitInstruction,
    /// Compute price instruction could not be parsed.
    #[error("Invalid compute price instruction")]
    InvalidComputePriceInstruction,
    /// Token instruction could not be parsed.
    #[error("Invalid token instruction")]
    InvalidTokenInstruction,
    /// Sender account is missing from the transaction.
    #[error("Missing sender account in transaction")]
    MissingSenderAccount,
}

impl From<SolanaExactError> for PaymentVerificationError {
    fn from(e: SolanaExactError) -> Self {
        match e {
            SolanaExactError::TransactionDecoding(_) => Self::InvalidFormat(e.to_string()),
            SolanaExactError::MaxComputeUnitLimitExceeded
            | SolanaExactError::MaxComputeUnitPriceExceeded
            | SolanaExactError::TooFewInstructions
            | SolanaExactError::AdditionalInstructionsNotAllowed
            | SolanaExactError::InstructionCountExceedsMax(_)
            | SolanaExactError::BlockedProgram(_)
            | SolanaExactError::ProgramNotAllowed(_)
            | SolanaExactError::CreateATANotSupported
            | SolanaExactError::FeePayerIncludedInInstructionAccounts
            | SolanaExactError::NoInstructionAtIndex(_)
            | SolanaExactError::InvalidComputeLimitInstruction
            | SolanaExactError::NoAccountAtIndex(_)
            | SolanaExactError::InvalidTokenInstruction
            | SolanaExactError::EmptyInstructionAtIndex(_)
            | SolanaExactError::FeePayerTransferringFunds
            | SolanaExactError::MissingSenderAccount
            | SolanaExactError::InvalidComputePriceInstruction => {
                Self::TransactionSimulation(e.to_string())
            }
        }
    }
}

/// Error encoding a transaction to base64.
#[derive(Debug, thiserror::Error)]
#[error("Can not encode transaction to base64: {0}")]
pub struct TransactionToB64Error(pub String);

/// Error signing a transaction.
#[derive(Debug, thiserror::Error)]
#[error("Can not sign transaction: {0}")]
pub struct TransactionSignError(pub String);
