//! Type definitions for the Solana "exact" payment scheme.
//!
//! This module defines shared wire format types for SPL Token based payments
//! on Solana, along with version-specific type aliases in the [`v1`] and
//! [`v2`] sub-modules.

use r402::lit_str;
use r402::proto::PaymentVerificationError;
use serde::{Deserialize, Serialize};
use solana_pubkey::{Pubkey, pubkey};
use std::sync::LazyLock;

use crate::chain::Address;
#[cfg(feature = "facilitator")]
use crate::chain::{SolanaChainProviderError, SolanaChainProviderLike};

#[cfg(any(feature = "client", feature = "facilitator"))]
use r402::encoding::Base64Bytes;
#[cfg(feature = "facilitator")]
use solana_commitment_config::CommitmentConfig;
#[cfg(any(feature = "client", feature = "facilitator"))]
use solana_message::compiled_instruction::CompiledInstruction;
#[cfg(any(feature = "client", feature = "facilitator"))]
use solana_signature::Signature;
#[cfg(any(feature = "client", feature = "facilitator"))]
use solana_signer::Signer;
#[cfg(any(feature = "client", feature = "facilitator"))]
use solana_transaction::versioned::VersionedTransaction;

lit_str!(ExactScheme, "exact");

/// Phantom Lighthouse program ID - security program injected by Phantom wallet on mainnet
/// See: <https://github.com/coinbase/x402/issues/828>
pub static PHANTOM_LIGHTHOUSE_PROGRAM: LazyLock<Pubkey> = LazyLock::new(|| {
    "L2TExMFKdjpN9kozasaurPirfHy9P8sbXoAN1qA3S95"
        .parse()
        .expect("Invalid Lighthouse program ID")
});

/// Solana exact payment payload containing a serialized transaction.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExactSolanaPayload {
    /// Base64-encoded serialized Solana transaction.
    pub transaction: String,
}

/// Extra fields for Solana payment kind support info.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SupportedPaymentKindExtra {
    /// The fee payer address for this payment kind.
    pub fee_payer: Address,
}

/// Associated Token Account program public key.
pub const ATA_PROGRAM_PUBKEY: Pubkey = pubkey!("ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL");

/// Parsed instruction with its index and resolved account keys.
#[derive(Debug)]
#[cfg(any(feature = "client", feature = "facilitator"))]
pub struct InstructionInt {
    index: usize,
    instruction: CompiledInstruction,
    account_keys: Vec<Pubkey>,
}

/// Wrapper around a versioned Solana transaction with helper methods.
#[derive(Debug)]
#[cfg(any(feature = "client", feature = "facilitator"))]
pub struct TransactionInt {
    inner: VersionedTransaction,
}

#[cfg(any(feature = "client", feature = "facilitator"))]
impl TransactionInt {
    /// Creates a new transaction wrapper.
    #[must_use]
    pub const fn new(transaction: VersionedTransaction) -> Self {
        Self { inner: transaction }
    }

    /// Returns the inner transaction.
    #[must_use]
    pub const fn inner(&self) -> &VersionedTransaction {
        &self.inner
    }

    /// Returns the instruction at the given index.
    ///
    /// # Errors
    ///
    /// Returns [`SolanaExactError`] if the index is out of bounds.
    pub fn instruction(&self, index: usize) -> Result<InstructionInt, SolanaExactError> {
        let instruction = self
            .inner
            .message
            .instructions()
            .get(index)
            .cloned()
            .ok_or(SolanaExactError::NoInstructionAtIndex(index))?;
        let account_keys = self.inner.message.static_account_keys().to_vec();

        Ok(InstructionInt {
            index,
            instruction,
            account_keys,
        })
    }

    /// Checks if the transaction is fully signed.
    #[must_use]
    pub fn is_fully_signed(&self) -> bool {
        let num_required = self.inner.message.header().num_required_signatures;
        if self.inner.signatures.len() < num_required as usize {
            return false;
        }
        let default = Signature::default();
        for signature in &self.inner.signatures {
            if default.eq(signature) {
                return false;
            }
        }
        true
    }

    /// Signs the transaction using the chain provider.
    ///
    /// # Errors
    ///
    /// Returns [`SolanaChainProviderError`] if signing fails.
    #[cfg(feature = "facilitator")]
    pub fn sign<P: SolanaChainProviderLike>(
        self,
        provider: &P,
    ) -> Result<Self, SolanaChainProviderError> {
        let tx = provider.sign(self.inner)?;
        Ok(Self { inner: tx })
    }

    /// Signs the transaction with any Signer.
    ///
    /// # Errors
    ///
    /// Returns [`TransactionSignError`] if the signer is not in the required signers list.
    pub fn sign_with_keypair<S: Signer>(self, signer: &S) -> Result<Self, TransactionSignError> {
        let mut tx = self.inner;
        let msg_bytes = tx.message.serialize();
        let signature = signer
            .try_sign_message(msg_bytes.as_slice())
            .map_err(|e| TransactionSignError(format!("{e}")))?;

        let num_required = tx.message.header().num_required_signatures as usize;
        let static_keys = tx.message.static_account_keys();

        let pos = static_keys[..num_required]
            .iter()
            .position(|k| *k == signer.pubkey())
            .ok_or_else(|| {
                TransactionSignError("Signer not found in required signers".to_string())
            })?;

        if tx.signatures.len() < num_required {
            tx.signatures.resize(num_required, Signature::default());
        }
        tx.signatures[pos] = signature;
        Ok(Self { inner: tx })
    }

    /// Sends the transaction and waits for confirmation.
    ///
    /// # Errors
    ///
    /// Returns [`SolanaChainProviderError`] if sending or confirmation fails.
    #[cfg(feature = "facilitator")]
    #[allow(clippy::needless_pass_by_value, clippy::future_not_send)]
    pub async fn send_and_confirm<P: SolanaChainProviderLike>(
        &self,
        provider: &P,
        commitment_config: CommitmentConfig,
    ) -> Result<Signature, SolanaChainProviderError> {
        provider
            .send_and_confirm(&self.inner, commitment_config)
            .await
    }

    /// Encodes the transaction to base64.
    ///
    /// # Errors
    ///
    /// Returns [`TransactionToB64Error`] if serialization or encoding fails.
    pub fn as_base64(&self) -> Result<String, TransactionToB64Error> {
        let bytes =
            bincode::serialize(&self.inner).map_err(|e| TransactionToB64Error(format!("{e}")))?;
        let base64_bytes = Base64Bytes::encode(bytes);
        let string = String::from_utf8(base64_bytes.0.into_owned())
            .map_err(|e| TransactionToB64Error(format!("{e}")))?;
        Ok(string)
    }
}

#[cfg(any(feature = "client", feature = "facilitator"))]
impl InstructionInt {
    /// Checks if the instruction has data.
    #[must_use]
    pub const fn has_data(&self) -> bool {
        !self.instruction.data.is_empty()
    }

    /// Checks if the instruction has accounts.
    #[must_use]
    pub const fn has_accounts(&self) -> bool {
        !self.instruction.accounts.is_empty()
    }

    /// Returns the instruction data as a slice.
    #[must_use]
    pub const fn data_slice(&self) -> &[u8] {
        self.instruction.data.as_slice()
    }

    /// Asserts that the instruction is not empty.
    ///
    /// # Errors
    ///
    /// Returns [`SolanaExactError`] if the instruction is empty.
    pub const fn assert_not_empty(&self) -> Result<(), SolanaExactError> {
        if !self.has_data() || !self.has_accounts() {
            return Err(SolanaExactError::EmptyInstructionAtIndex(self.index));
        }
        Ok(())
    }

    /// Returns the program ID of the instruction.
    #[must_use]
    pub fn program_id(&self) -> Pubkey {
        *self.instruction.program_id(self.account_keys.as_slice())
    }

    /// Returns the account public key at the given index.
    ///
    /// # Errors
    ///
    /// Returns [`SolanaExactError`] if the index is out of bounds.
    pub fn account(&self, index: u8) -> Result<Pubkey, SolanaExactError> {
        let account_index = self
            .instruction
            .accounts
            .get(index as usize)
            .copied()
            .ok_or(SolanaExactError::NoAccountAtIndex(index))?;
        let pubkey = self
            .account_keys
            .get(account_index as usize)
            .copied()
            .ok_or(SolanaExactError::NoAccountAtIndex(index))?;
        Ok(pubkey)
    }
}

/// Error encoding a transaction to base64.
#[derive(Debug, thiserror::Error)]
#[error("Can not encode transaction to base64: {0}")]
pub struct TransactionToB64Error(String);

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

/// Error signing a transaction.
#[derive(Debug, thiserror::Error)]
#[error("Can not sign transaction: {0}")]
pub struct TransactionSignError(pub String);

/// V1-specific wire format type aliases for Solana exact scheme.
///
/// V1 uses network names (e.g., "solana-mainnet") for chain identification.
pub mod v1 {
    use r402::proto::U64String;
    use r402::proto::v1 as proto_v1;

    use super::{ExactScheme, ExactSolanaPayload, SupportedPaymentKindExtra};
    use crate::chain::Address;

    /// Type alias for V1 verify requests using the exact Solana payment scheme.
    pub type VerifyRequest = proto_v1::VerifyRequest<PaymentPayload, PaymentRequirements>;

    /// Type alias for V1 settle requests (same structure as verify requests).
    pub type SettleRequest = VerifyRequest;

    /// Type alias for V1 payment payloads with Solana-specific data.
    pub type PaymentPayload = proto_v1::PaymentPayload<ExactScheme, ExactSolanaPayload>;

    /// Type alias for V1 payment requirements with Solana-specific types.
    pub type PaymentRequirements =
        proto_v1::PaymentRequirements<ExactScheme, U64String, Address, SupportedPaymentKindExtra>;
}

/// V2-specific wire format type aliases for Solana exact scheme.
///
/// V2 uses CAIP-2 chain IDs (e.g., `solana:5eykt4UsFv8P8NJdTREpY1vzqKqZKvdp`)
/// for chain identification and embeds requirements directly in the payload.
pub mod v2 {
    use r402::proto::U64String;
    use r402::proto::v2 as proto_v2;

    use super::{ExactScheme, ExactSolanaPayload, SupportedPaymentKindExtra};
    use crate::chain::Address;

    /// Type alias for V2 verify requests using the exact Solana payment scheme.
    pub type VerifyRequest = proto_v2::VerifyRequest<PaymentPayload, PaymentRequirements>;

    /// Type alias for V2 settle requests (same structure as verify requests).
    pub type SettleRequest = VerifyRequest;

    /// Type alias for V2 payment payloads with embedded requirements and Solana-specific data.
    pub type PaymentPayload = proto_v2::PaymentPayload<PaymentRequirements, ExactSolanaPayload>;

    /// Type alias for V2 payment requirements with Solana-specific types.
    pub type PaymentRequirements =
        proto_v2::PaymentRequirements<ExactScheme, U64String, Address, SupportedPaymentKindExtra>;
}
