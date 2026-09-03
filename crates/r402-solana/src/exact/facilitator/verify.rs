//! Payment verification for the Solana exact scheme.
//!
//! Compute-budget checks, instruction validation, and transfer verification.

use r402_protocol::{Base64Bytes, ChainProvider, VerificationError};
use solana_client::rpc_config::RpcSimulateTransactionConfig;
use solana_commitment_config::CommitmentConfig;
use solana_compute_budget_interface::ID as ComputeBudgetInstructionId;
use solana_pubkey::Pubkey;
use solana_transaction::versioned::VersionedTransaction;

use super::config::SolanaExactFacilitatorConfig;
use crate::chain::Address;
use crate::chain::provider::SolanaChainProviderLike;
use crate::exact::ATA_PROGRAM_PUBKEY;
use crate::exact::error::SolanaExactError;
use crate::exact::payload::{self, TransactionInt};

/// Result of a successful transfer verification.
#[derive(Debug)]
pub struct VerifyTransferResult {
    /// The payer address.
    pub payer: Address,
    /// The verified transaction.
    pub transaction: VersionedTransaction,
    /// On-chain `TransferChecked` amount in token base units.
    ///
    /// May exceed [`TransferRequirement::amount`] when the buyer overpays
    /// (allowed by `scheme_exact_svm.md` §1.4 and the Go/TS facilitators).
    /// Underpayment is rejected before this struct is constructed.
    pub amount: u64,
}

/// Returns `true` when `actual` meets the required transfer amount.
///
/// Aligns with x402 `scheme_exact_svm.md` §1.4, the Go facilitator
/// (`verifyTransferInstruction`: reject only when `amount < required`),
/// and the TypeScript smart-wallet matcher (`t.amount >= requiredAmount`).
///
/// Overpayment is tolerated for smart-wallet fee-rounding; underpayment is not.
#[must_use]
#[inline]
pub const fn transfer_amount_meets_requirement(actual: u64, required: u64) -> bool {
    actual >= required
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
    #[allow(clippy::indexing_slicing, reason = "length == 5 checked above")]
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
    #[allow(clippy::indexing_slicing, reason = "length == 9 checked above")]
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

        #[allow(
            clippy::excessive_nesting,
            reason = "program validation requires nested conditionals"
        )]
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

/// Verifies a transfer request against on-chain state.
///
/// # Errors
///
/// Returns [`VerificationError`] if the transfer is invalid.
pub async fn verify_transfer<P: SolanaChainProviderLike + ChainProvider>(
    provider: &P,
    request: &payload::v2::VerifyRequest,
    config: &SolanaExactFacilitatorConfig,
) -> Result<VerifyTransferResult, VerificationError> {
    let payload = &request.payment_payload;
    let requirements = &request.payment_requirements;

    let accepted = &payload.accepted;
    if !(accepted.scheme == requirements.scheme
        && accepted.network == requirements.network
        && accepted.amount == requirements.amount
        && accepted.asset == requirements.asset
        && accepted.pay_to == requirements.pay_to)
    {
        return Err(VerificationError::AcceptedRequirementsMismatch);
    }

    let chain_id = provider.chain_id();
    let payload_chain_id = &accepted.network;
    if payload_chain_id != &chain_id {
        return Err(VerificationError::UnsupportedChain);
    }

    // F-025: enforce that the buyer pinned this payment to a fee payer the
    // facilitator actually controls. Without this check, a buyer could spoof
    // any address in `extra.feePayer`; the facilitator would still accept and
    // submit the transaction (bound to its own keypair) as if it had been
    // explicitly addressed, defeating the spec §SVM scheme contract that the
    // buyer-signed payload binds to a specific facilitator.
    let declared_extra = accepted.extra.as_ref().ok_or_else(|| {
        VerificationError::InvalidFormat(
            "missing paymentRequirements.extra: spec §SVM exact requires `feePayer`".into(),
        )
    })?;
    let configured_signers = provider.signer_addresses();
    let declared_fee_payer = declared_extra.fee_payer.to_string();
    if !configured_signers.iter().any(|s| s == &declared_fee_payer) {
        return Err(VerificationError::InvalidFormat(format!(
            "declared feePayer {declared_fee_payer} is not managed by this facilitator",
        )));
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
/// Tries Path 1 (static layout) first. When
/// [`SolanaExactFacilitatorConfig::enable_smart_wallet_verification`] is set
/// and Path 1 fails for a recoverable layout reason, falls through to Path 2
/// (outcome-based matching per `scheme_exact_svm.md` §3.2).
///
/// # Errors
///
/// Returns [`VerificationError`] if verification fails.
pub async fn verify_transaction<P: SolanaChainProviderLike>(
    provider: &P,
    transaction_b64_string: String,
    transfer_requirement: &TransferRequirement<'_>,
    config: &SolanaExactFacilitatorConfig,
) -> Result<VerifyTransferResult, VerificationError> {
    let bytes = Base64Bytes::from(transaction_b64_string.as_bytes())
        .decode()
        .map_err(|e| SolanaExactError::TransactionDecoding(e.to_string()))?;
    let transaction = bincode::deserialize::<VersionedTransaction>(bytes.as_slice())
        .map_err(|e| SolanaExactError::TransactionDecoding(e.to_string()))?;

    match verify_transaction_path1(provider, transaction.clone(), transfer_requirement, config)
        .await
    {
        Ok(ok) => Ok(ok),
        Err(path1_err)
            if config.enable_smart_wallet_verification
                && super::smart_wallet::is_path1_layout_recoverable(&path1_err) =>
        {
            #[cfg(feature = "telemetry")]
            tracing::debug!(
                path1 = %path1_err,
                "Path 1 layout failure; attempting Path 2 smart-wallet verification"
            );
            verify_transaction_path2(provider, transaction, transfer_requirement).await
        }
        Err(e) => Err(e),
    }
}

/// Path 1 — static layout verification (standard wallets).
async fn verify_transaction_path1<P: SolanaChainProviderLike>(
    provider: &P,
    transaction: VersionedTransaction,
    transfer_requirement: &TransferRequirement<'_>,
    config: &SolanaExactFacilitatorConfig,
) -> Result<VerifyTransferResult, VerificationError> {
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

    // F-030: always reject transactions that touch the facilitator's fee
    // payer account in any non-fee-paying role.
    assert_fee_payer_isolated(&transaction, provider.pubkey())?;

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
    Ok(VerifyTransferResult {
        payer,
        transaction,
        amount: transfer_instruction.amount,
    })
}

/// Path 2 — simulation / outcome-based smart-wallet verification.
///
/// Matches `TransferChecked` instructions anywhere in the top-level message
/// (and, once RPC inners are plumbed through the provider, CPI traces).
/// Full CPI extraction from `simulateTransaction` inner instructions is the
/// next hardening step; top-level multi-ix wallets already work here.
async fn verify_transaction_path2<P: SolanaChainProviderLike>(
    provider: &P,
    transaction: VersionedTransaction,
    transfer_requirement: &TransferRequirement<'_>,
) -> Result<VerifyTransferResult, VerificationError> {
    assert_fee_payer_isolated(&transaction, provider.pubkey())?;

    let mut transfers = super::smart_wallet::extract_top_level_transfers(&transaction);
    // Simulate with inner instructions requested so the RPC records CPI
    // traces (used by operators / future extractors). Path 2 still requires
    // a successful simulation before accepting the payment.
    let tx = TransactionInt::new(transaction.clone()).sign(provider)?;
    let cfg = RpcSimulateTransactionConfig {
        sig_verify: false,
        replace_recent_blockhash: false,
        commitment: Some(CommitmentConfig::confirmed()),
        encoding: None,
        accounts: None,
        inner_instructions: true,
        min_context_slot: None,
    };
    provider
        .simulate_transaction_with_config(tx.inner(), cfg)
        .await?;

    // If no top-level TransferChecked was found, surface a clear Path 2 error.
    // CPI-only wallets need the provider to return sim.inner_instructions
    // (tracked as follow-up: plumb RpcSimulateTransactionResult through
    // SolanaChainProviderLike).
    if transfers.is_empty() {
        return Err(VerificationError::InvalidFormat(
            "smart_wallet_no_transfer_in_simulation: no top-level TransferChecked; \
             CPI-only smart wallets require inner-instruction plumbing (enable \
             provider detailed simulation)"
                .into(),
        ));
    }

    let fee_payers = [provider.pubkey()];
    let matched = super::smart_wallet::match_required_transfer(
        &transfers,
        transfer_requirement,
        &fee_payers,
    )?;
    // Silence unused_mut if we later push CPI transfers into `transfers`.
    let _ = &mut transfers;

    Ok(VerifyTransferResult {
        payer: matched.authority.into(),
        transaction,
        amount: matched.amount,
    })
}

fn assert_fee_payer_isolated(
    transaction: &VersionedTransaction,
    fee_payer_pubkey: Pubkey,
) -> Result<(), VerificationError> {
    for instruction in transaction.message.instructions() {
        for account_idx in &instruction.accounts {
            let account = transaction
                .message
                .static_account_keys()
                .get(*account_idx as usize)
                .ok_or(SolanaExactError::NoAccountAtIndex(*account_idx))?;

            #[allow(
                clippy::excessive_nesting,
                reason = "nested iteration over instructions and accounts"
            )]
            if *account == fee_payer_pubkey {
                return Err(SolanaExactError::FeePayerIncludedInInstructionAccounts.into());
            }
        }
    }
    Ok(())
}

/// Verifies the SPL Token transfer instruction at the given index.
///
/// # Errors
///
/// Returns [`VerificationError`] if the transfer instruction is invalid.
pub async fn verify_transfer_instruction<P: SolanaChainProviderLike>(
    provider: &P,
    transaction: &VersionedTransaction,
    instruction_index: usize,
    transfer_requirement: &TransferRequirement<'_>,
) -> Result<TransferCheckedInstruction, VerificationError> {
    let tx = TransactionInt::new(transaction.clone());
    let instruction = tx.instruction(instruction_index)?;
    instruction.assert_not_empty()?;
    let program_id = instruction.program_id();
    // Both spl_token and the Token-2022 interface share the same
    // `TransferChecked` instruction layout, so we use spl_token's unpack
    // for both and only differentiate by program ID. The Token-2022 ID
    // comes from the interface-only crate to avoid pulling in the full
    // processor (and its broken `token_group/processor.rs` upstream).
    let token_program = if spl_token::ID.eq(&program_id) {
        spl_token::ID
    } else if spl_token_2022_interface::ID.eq(&program_id) {
        spl_token_2022_interface::ID
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
        return Err(VerificationError::AssetMismatch);
    }

    let expected_token_program = transfer_checked_instruction.token_program;
    let (ata, _) = Pubkey::find_program_address(
        &[
            transfer_requirement.pay_to.as_ref(),
            expected_token_program.as_ref(),
            transfer_requirement.asset.as_ref(),
        ],
        &ATA_PROGRAM_PUBKEY,
    );
    if transfer_checked_instruction.destination != ata {
        return Err(VerificationError::RecipientMismatch);
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
        return Err(VerificationError::RecipientMismatch);
    }
    // Spec §1.4 / Go / TS: amount must be at least the required amount.
    // Overpayment is allowed (wallet fee rounding); underpayment is rejected.
    if !transfer_amount_meets_requirement(
        transfer_checked_instruction.amount,
        transfer_requirement.amount,
    ) {
        return Err(VerificationError::InvalidPaymentAmount);
    }
    Ok(transfer_checked_instruction)
}

#[cfg(test)]
mod tests {
    use super::transfer_amount_meets_requirement;

    #[test]
    fn exact_amount_meets_requirement() {
        assert!(transfer_amount_meets_requirement(1_000_000, 1_000_000));
    }

    #[test]
    fn overpayment_meets_requirement() {
        // scheme_exact_svm.md §1.4 + Go/TS: actual >= required.
        assert!(transfer_amount_meets_requirement(1_000_001, 1_000_000));
        assert!(transfer_amount_meets_requirement(u64::MAX, 1));
    }

    #[test]
    fn underpayment_fails_requirement() {
        assert!(!transfer_amount_meets_requirement(999_999, 1_000_000));
        assert!(!transfer_amount_meets_requirement(0, 1));
    }

    #[test]
    fn zero_required_always_meets() {
        assert!(transfer_amount_meets_requirement(0, 0));
        assert!(transfer_amount_meets_requirement(1, 0));
    }
}
