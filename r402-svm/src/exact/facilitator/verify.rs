//! Payment verification and settlement logic for the Solana exact scheme.
//!
//! Contains compute budget checks, instruction validation, transfer verification,
//! and the settlement function.

use r402::chain::{ChainId, ChainProviderOps};
use r402::encoding::Base64Bytes;
use r402::proto::PaymentVerificationError;
use solana_client::rpc_config::RpcSimulateTransactionConfig;
use solana_client::rpc_response::{TransactionError, UiTransactionError};
use solana_commitment_config::CommitmentConfig;
use solana_compute_budget_interface::ID as ComputeBudgetInstructionId;
use solana_pubkey::Pubkey;
use solana_signature::Signature;
use solana_transaction::versioned::VersionedTransaction;

#[cfg(feature = "telemetry")]
use tracing_core::Level;

use super::config::SolanaExactFacilitatorConfig;
use crate::chain::Address;
use crate::chain::provider::{SolanaChainProviderError, SolanaChainProviderLike};
use crate::exact::ATA_PROGRAM_PUBKEY;
use crate::exact::error::SolanaExactError;
use crate::exact::types::{self, TransactionInt};

/// Result of a successful transfer verification.
#[derive(Debug)]
pub struct VerifyTransferResult {
    /// The payer address.
    pub payer: Address,
    /// The verified transaction.
    pub transaction: VersionedTransaction,
}

/// Parsed SPL Token `TransferChecked` instruction fields.
#[derive(Debug, Clone, Copy)]
pub struct TransferCheckedInstruction {
    /// Transfer amount in token base units.
    pub amount: u64,
    /// Source token account.
    pub source: Pubkey,
    /// Token mint address.
    pub mint: Pubkey,
    /// Destination token account.
    pub destination: Pubkey,
    /// Authority (signer) of the transfer.
    pub authority: Pubkey,
    /// SPL Token program ID (Token or Token-2022).
    pub token_program: Pubkey,
}

/// Required fields for validating a transfer.
#[derive(Debug)]
pub struct TransferRequirement<'a> {
    /// Expected asset (mint) address.
    pub asset: &'a Address,
    /// Expected recipient address.
    pub pay_to: &'a Address,
    /// Expected transfer amount in base units.
    pub amount: u64,
}

/// Verifies the compute unit limit instruction at the given index.
///
/// # Errors
///
/// Returns [`SolanaExactError`] if the instruction is invalid.
pub fn verify_compute_limit_instruction(
    transaction: &VersionedTransaction,
    instruction_index: usize,
) -> Result<u32, SolanaExactError> {
    let instructions = transaction.message.instructions();
    let instruction = instructions
        .get(instruction_index)
        .ok_or(SolanaExactError::NoInstructionAtIndex(instruction_index))?;
    let account = instruction.program_id(transaction.message.static_account_keys());
    let data = instruction.data.as_slice();

    if ComputeBudgetInstructionId.ne(account)
        || data.first().copied().unwrap_or(0) != 2
        || data.len() != 5
    {
        return Err(SolanaExactError::InvalidComputeLimitInstruction);
    }

    let mut buf = [0u8; 4];
    buf.copy_from_slice(&data[1..5]);
    let compute_units = u32::from_le_bytes(buf);

    Ok(compute_units)
}

/// Verifies the compute unit price instruction at the given index.
///
/// # Errors
///
/// Returns [`SolanaExactError`] if the instruction is invalid or price exceeds max.
pub fn verify_compute_price_instruction(
    max_compute_unit_price: u64,
    transaction: &VersionedTransaction,
    instruction_index: usize,
) -> Result<(), SolanaExactError> {
    let instructions = transaction.message.instructions();
    let instruction = instructions
        .get(instruction_index)
        .ok_or(SolanaExactError::NoInstructionAtIndex(instruction_index))?;
    let account = instruction.program_id(transaction.message.static_account_keys());
    let compute_budget = solana_compute_budget_interface::ID;
    let data = instruction.data.as_slice();
    if compute_budget.ne(account) || data.first().copied().unwrap_or(0) != 3 || data.len() != 9 {
        return Err(SolanaExactError::InvalidComputePriceInstruction);
    }
    let mut buf = [0u8; 8];
    buf.copy_from_slice(&data[1..]);
    let microlamports = u64::from_le_bytes(buf);
    if microlamports > max_compute_unit_price {
        return Err(SolanaExactError::MaxComputeUnitPriceExceeded);
    }
    Ok(())
}

/// Validates the instruction structure of the transaction.
///
/// # Errors
///
/// Returns [`SolanaExactError`] if instruction validation fails.
pub fn validate_instructions(
    transaction: &VersionedTransaction,
    config: &SolanaExactFacilitatorConfig,
) -> Result<(), SolanaExactError> {
    let instructions = transaction.message.instructions();

    if instructions.len() < 3 {
        return Err(SolanaExactError::TooFewInstructions);
    }

    if instructions.len() > config.max_instruction_count {
        return Err(SolanaExactError::InstructionCountExceedsMax(
            config.max_instruction_count,
        ));
    }

    let ix2_program = get_program_id(transaction, 2);
    if ix2_program == Some(ATA_PROGRAM_PUBKEY) {
        return Err(SolanaExactError::CreateATANotSupported);
    }

    if instructions.len() > 3 {
        if !config.allow_additional_instructions {
            return Err(SolanaExactError::AdditionalInstructionsNotAllowed);
        }

        for i in 3..instructions.len() {
            if let Some(program_id) = get_program_id(transaction, i) {
                if config.is_blocked(&program_id) {
                    return Err(SolanaExactError::BlockedProgram(program_id));
                }

                if !config.is_allowed(&program_id) {
                    return Err(SolanaExactError::ProgramNotAllowed(program_id));
                }
            }
        }
    }

    Ok(())
}

fn get_program_id(transaction: &VersionedTransaction, index: usize) -> Option<Pubkey> {
    let instruction = transaction.message.instructions().get(index)?;
    let account_keys = transaction.message.static_account_keys();
    Some(*instruction.program_id(account_keys))
}

/// Verifies a V1 transfer request against on-chain state.
///
/// # Errors
///
/// Returns [`PaymentVerificationError`] if the transfer is invalid.
#[allow(clippy::future_not_send)]
pub async fn verify_v1_transfer<P: SolanaChainProviderLike + ChainProviderOps>(
    provider: &P,
    request: &types::v1::VerifyRequest,
    config: &SolanaExactFacilitatorConfig,
) -> Result<VerifyTransferResult, PaymentVerificationError> {
    let payload = &request.payment_payload;
    let requirements = &request.payment_requirements;

    let chain_id = provider.chain_id();
    let payload_chain_id = ChainId::from_network_name(&payload.network)
        .ok_or(PaymentVerificationError::UnsupportedChain)?;
    if payload_chain_id != chain_id {
        return Err(PaymentVerificationError::ChainIdMismatch);
    }
    let requirements_chain_id = ChainId::from_network_name(&requirements.network)
        .ok_or(PaymentVerificationError::UnsupportedChain)?;
    if requirements_chain_id != chain_id {
        return Err(PaymentVerificationError::ChainIdMismatch);
    }
    let transaction_b64_string = payload.payload.transaction.clone();
    let transfer_requirement = TransferRequirement {
        pay_to: &requirements.pay_to,
        asset: &requirements.asset,
        amount: requirements.max_amount_required.inner(),
    };
    verify_transaction(
        provider,
        transaction_b64_string,
        &transfer_requirement,
        config,
    )
    .await
}

/// Verifies a V2 transfer request against on-chain state.
///
/// # Errors
///
/// Returns [`PaymentVerificationError`] if the transfer is invalid.
#[allow(clippy::future_not_send)]
pub async fn verify_v2_transfer<P: SolanaChainProviderLike + ChainProviderOps>(
    provider: &P,
    request: &types::v2::VerifyRequest,
    config: &SolanaExactFacilitatorConfig,
) -> Result<VerifyTransferResult, PaymentVerificationError> {
    let payload = &request.payment_payload;
    let requirements = &request.payment_requirements;

    let accepted = &payload.accepted;
    if accepted != requirements {
        return Err(PaymentVerificationError::AcceptedRequirementsMismatch);
    }

    let chain_id = provider.chain_id();
    let payload_chain_id = &accepted.network;
    if payload_chain_id != &chain_id {
        return Err(PaymentVerificationError::UnsupportedChain);
    }
    let transaction_b64_string = payload.payload.transaction.clone();
    let transfer_requirement = TransferRequirement {
        pay_to: &requirements.pay_to,
        asset: &requirements.asset,
        amount: requirements.amount.inner(),
    };
    verify_transaction(
        provider,
        transaction_b64_string,
        &transfer_requirement,
        config,
    )
    .await
}

/// Verifies a base64-encoded transaction against requirements.
///
/// # Errors
///
/// Returns [`PaymentVerificationError`] if verification fails.
#[allow(clippy::future_not_send)]
pub async fn verify_transaction<P: SolanaChainProviderLike>(
    provider: &P,
    transaction_b64_string: String,
    transfer_requirement: &TransferRequirement<'_>,
    config: &SolanaExactFacilitatorConfig,
) -> Result<VerifyTransferResult, PaymentVerificationError> {
    let bytes = Base64Bytes::from(transaction_b64_string.as_bytes())
        .decode()
        .map_err(|e| SolanaExactError::TransactionDecoding(e.to_string()))?;
    let transaction = bincode::deserialize::<VersionedTransaction>(bytes.as_slice())
        .map_err(|e| SolanaExactError::TransactionDecoding(e.to_string()))?;

    let compute_units = verify_compute_limit_instruction(&transaction, 0)?;
    if compute_units > provider.max_compute_unit_limit() {
        return Err(SolanaExactError::MaxComputeUnitLimitExceeded.into());
    }
    #[cfg(feature = "telemetry")]
    tracing::debug!(compute_units = compute_units, "Verified compute unit limit");
    verify_compute_price_instruction(provider.max_compute_unit_price(), &transaction, 1)?;

    validate_instructions(&transaction, config)?;

    let transfer_instruction =
        verify_transfer_instruction(provider, &transaction, 2, transfer_requirement).await?;

    if config.require_fee_payer_not_in_instructions {
        let fee_payer_pubkey = provider.pubkey();
        for instruction in transaction.message.instructions() {
            for account_idx in &instruction.accounts {
                let account = transaction
                    .message
                    .static_account_keys()
                    .get(*account_idx as usize)
                    .ok_or(SolanaExactError::NoAccountAtIndex(*account_idx))?;

                if *account == fee_payer_pubkey {
                    return Err(SolanaExactError::FeePayerIncludedInInstructionAccounts.into());
                }
            }
        }
    }

    let tx = TransactionInt::new(transaction.clone()).sign(provider)?;
    let cfg = RpcSimulateTransactionConfig {
        sig_verify: false,
        replace_recent_blockhash: false,
        commitment: Some(CommitmentConfig::confirmed()),
        encoding: None,
        accounts: None,
        inner_instructions: false,
        min_context_slot: None,
    };
    provider
        .simulate_transaction_with_config(tx.inner(), cfg)
        .await?;
    let payer: Address = transfer_instruction.authority.into();
    Ok(VerifyTransferResult { payer, transaction })
}

/// Verifies the SPL Token transfer instruction at the given index.
///
/// # Errors
///
/// Returns [`PaymentVerificationError`] if the transfer instruction is invalid.
#[allow(clippy::future_not_send)]
pub async fn verify_transfer_instruction<P: SolanaChainProviderLike>(
    provider: &P,
    transaction: &VersionedTransaction,
    instruction_index: usize,
    transfer_requirement: &TransferRequirement<'_>,
) -> Result<TransferCheckedInstruction, PaymentVerificationError> {
    let tx = TransactionInt::new(transaction.clone());
    let instruction = tx.instruction(instruction_index)?;
    instruction.assert_not_empty()?;
    let program_id = instruction.program_id();
    // Both spl_token and spl_token_2022 share the same instruction layout,
    // so we use spl_token's unpack for both and only differentiate by program ID.
    let token_program = if spl_token::ID.eq(&program_id) {
        spl_token::ID
    } else if spl_token_2022::ID.eq(&program_id) {
        spl_token_2022::ID
    } else {
        return Err(SolanaExactError::InvalidTokenInstruction.into());
    };
    let token_instruction =
        spl_token::instruction::TokenInstruction::unpack(instruction.data_slice())
            .map_err(|_| SolanaExactError::InvalidTokenInstruction)?;
    let spl_token::instruction::TokenInstruction::TransferChecked {
        amount,
        decimals: _,
    } = token_instruction
    else {
        return Err(SolanaExactError::InvalidTokenInstruction.into());
    };
    let transfer_checked_instruction = TransferCheckedInstruction {
        amount,
        source: instruction.account(0)?,
        mint: instruction.account(1)?,
        destination: instruction.account(2)?,
        authority: instruction.account(3)?,
        token_program,
    };

    let fee_payer_pubkey = provider.pubkey();
    if transfer_checked_instruction.authority == fee_payer_pubkey {
        return Err(SolanaExactError::FeePayerTransferringFunds.into());
    }

    if Address::new(transfer_checked_instruction.mint) != *transfer_requirement.asset {
        return Err(PaymentVerificationError::AssetMismatch);
    }

    let token_program = transfer_checked_instruction.token_program;
    let (ata, _) = Pubkey::find_program_address(
        &[
            transfer_requirement.pay_to.as_ref(),
            token_program.as_ref(),
            transfer_requirement.asset.as_ref(),
        ],
        &ATA_PROGRAM_PUBKEY,
    );
    if transfer_checked_instruction.destination != ata {
        return Err(PaymentVerificationError::RecipientMismatch);
    }
    let accounts = provider
        .get_multiple_accounts(&[transfer_checked_instruction.source, ata])
        .await?;
    let is_sender_missing = accounts.first().cloned().is_none_or(|a| a.is_none());
    if is_sender_missing {
        return Err(SolanaExactError::MissingSenderAccount.into());
    }
    let is_receiver_missing = accounts.get(1).cloned().is_none_or(|a| a.is_none());
    if is_receiver_missing {
        return Err(PaymentVerificationError::RecipientMismatch);
    }
    let instruction_amount = transfer_checked_instruction.amount;
    if instruction_amount != transfer_requirement.amount {
        return Err(PaymentVerificationError::InvalidPaymentAmount);
    }
    Ok(transfer_checked_instruction)
}

/// Settles a verified transaction by signing and sending it.
///
/// # Errors
///
/// Returns [`SolanaChainProviderError`] if settling fails.
#[allow(clippy::future_not_send)]
pub async fn settle_transaction<P: SolanaChainProviderLike>(
    provider: &P,
    verification: VerifyTransferResult,
) -> Result<Signature, SolanaChainProviderError> {
    let tx = TransactionInt::new(verification.transaction).sign(provider)?;
    if !tx.is_fully_signed() {
        #[cfg(feature = "telemetry")]
        tracing::event!(Level::WARN, status = "failed", "undersigned transaction");
        return Err(SolanaChainProviderError::InvalidTransaction(
            UiTransactionError::from(TransactionError::SignatureFailure),
        ));
    }
    let tx_sig = tx
        .send_and_confirm(provider, CommitmentConfig::confirmed())
        .await?;
    Ok(tx_sig)
}
