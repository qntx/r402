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
use super::smart_wallet::ObservedTransfer;
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
    /// Path 2 matched `TransferChecked`. `None` on Path 1.
    pub matched_transfer: Option<ObservedTransfer>,
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

    let max = config.effective_max_instruction_count();
    if instructions.len() > max {
        return Err(SolanaExactError::InstructionCountExceedsMax(max));
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
            verify_transaction_path2(provider, transaction, transfer_requirement, config).await
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
        matched_transfer: None,
    })
}

/// Path 2 — official order: caps → allowlist → ALT isolation → compute budget
/// → simulate inners → exactly-one match (spec §3.2).
async fn verify_transaction_path2<P: SolanaChainProviderLike>(
    provider: &P,
    transaction: VersionedTransaction,
    transfer_requirement: &TransferRequirement<'_>,
    config: &SolanaExactFacilitatorConfig,
) -> Result<VerifyTransferResult, VerificationError> {
    super::smart_wallet::assert_path2_provider(provider, config)?;
    super::smart_wallet::assert_path2_program_allowlist(&transaction, config)?;
    let keys = super::smart_wallet::resolve_loaded_account_keys(provider, &transaction).await?;
    super::smart_wallet::assert_fee_payer_isolated_from_keys(
        &transaction,
        &keys,
        provider.pubkey(),
    )?;
    super::smart_wallet::validate_path2_compute_budget(&transaction, &keys, config)?;

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
    let sim = provider
        .simulate_transaction_with_config(tx.inner(), cfg)
        .await
        .map_err(|e| {
            VerificationError::from_wire(&format!(
                "{}: {e}",
                super::smart_wallet::simulation_failed_code()
            ))
        })?;

    let mut account_keys = keys;
    let loaded = super::smart_wallet::resolved_account_keys(&transaction, &sim.loaded_addresses);
    if loaded.len() > account_keys.len() {
        account_keys = loaded;
    }
    let mut transfers =
        super::smart_wallet::extract_top_level_transfers_with_keys(&transaction, &account_keys);
    transfers.extend(
        super::smart_wallet::extract_transfers_from_inner_instructions(
            &sim.inner_instructions,
            &account_keys,
        ),
    );

    let fee_payers = [provider.pubkey()];
    let matched = super::smart_wallet::match_required_transfer(
        &transfers,
        transfer_requirement,
        &fee_payers,
    )?;

    Ok(VerifyTransferResult {
        payer: matched.authority.into(),
        transaction,
        amount: matched.amount,
        matched_transfer: Some(matched),
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
    use std::future::Future;
    use std::sync::Arc;

    use solana_account::Account;
    use solana_client::nonblocking::rpc_client::RpcClient;
    use solana_client::rpc_response::UiInnerInstructions;
    use solana_commitment_config::CommitmentConfig;
    use solana_compute_budget_interface::ID as COMPUTE_BUDGET;
    use solana_message::{Message, VersionedMessage};
    use solana_signature::Signature;
    use solana_transaction::Instruction;

    use super::*;
    use crate::chain::TOKEN_PROGRAM_ID;
    use crate::chain::provider::{
        SignatureConfirm, SimulateTransactionResult, SolanaChainProviderError,
        SolanaChainProviderLike,
    };
    use crate::exact::{PHANTOM_LIGHTHOUSE_PROGRAM, SPL_MEMO_PROGRAM, SolanaExactError};

    struct Path2Stub {
        pubkey: Pubkey,
        inner: Vec<UiInnerInstructions>,
        loaded: solana_client::rpc_response::UiLoadedAddresses,
        smart_wallet: bool,
    }

    impl SolanaChainProviderLike for Path2Stub {
        fn simulate_transaction_with_config(
            &self,
            _tx: &VersionedTransaction,
            _cfg: RpcSimulateTransactionConfig,
        ) -> impl Future<Output = Result<SimulateTransactionResult, SolanaChainProviderError>> + Send
        {
            std::future::ready(Ok(SimulateTransactionResult {
                inner_instructions: self.inner.clone(),
                loaded_addresses: self.loaded.clone(),
                ..SimulateTransactionResult::default()
            }))
        }

        fn get_multiple_accounts(
            &self,
            _pubkeys: &[Pubkey],
        ) -> impl Future<Output = Result<Vec<Option<Account>>, SolanaChainProviderError>> + Send
        {
            std::future::ready(Ok(Vec::new()))
        }

        fn max_compute_unit_limit(&self) -> u32 {
            400_000
        }

        fn max_compute_unit_price(&self) -> u64 {
            1
        }

        fn pubkey(&self) -> Pubkey {
            self.pubkey
        }

        fn fee_payer(&self) -> Address {
            Address::new(self.pubkey)
        }

        fn sign(
            &self,
            tx: VersionedTransaction,
        ) -> Result<VersionedTransaction, SolanaChainProviderError> {
            Ok(tx)
        }

        fn send_and_confirm(
            &self,
            _tx: &VersionedTransaction,
            _commitment_config: CommitmentConfig,
        ) -> impl Future<Output = Result<Signature, SolanaChainProviderError>> + Send {
            std::future::ready(Err(SolanaChainProviderError::Custom("unused".into())))
        }

        fn rpc_client(&self) -> Arc<RpcClient> {
            Arc::new(RpcClient::new("http://127.0.0.1:9".to_owned()))
        }

        fn send(
            &self,
            _tx: &VersionedTransaction,
        ) -> impl Future<Output = Result<Signature, SolanaChainProviderError>> + Send {
            std::future::ready(Err(SolanaChainProviderError::Custom("unused".into())))
        }

        fn confirm_signature(
            &self,
            _signature: &Signature,
            _commitment_config: CommitmentConfig,
        ) -> impl Future<Output = Result<SignatureConfirm, SolanaChainProviderError>> + Send
        {
            std::future::ready(Ok(SignatureConfirm::Confirmed))
        }

        fn fetch_address_lookup_tables(
            &self,
            _tables: &[Pubkey],
        ) -> impl Future<
            Output = Result<
                std::collections::HashMap<Pubkey, Vec<Pubkey>>,
                SolanaChainProviderError,
            >,
        > + Send {
            std::future::ready(Ok(std::collections::HashMap::new()))
        }

        fn supports_smart_wallet(&self) -> bool {
            self.smart_wallet
        }
    }

    fn compiled_tx(programs: &[Pubkey]) -> VersionedTransaction {
        let payer = Pubkey::new_from_array([1u8; 32]);
        let ixs: Vec<Instruction> = programs
            .iter()
            .map(|program_id| Instruction {
                program_id: *program_id,
                accounts: Vec::new(),
                data: vec![0],
            })
            .collect();
        VersionedTransaction {
            signatures: vec![Signature::default()],
            message: VersionedMessage::Legacy(Message::new(&ixs, Some(&payer))),
        }
    }

    fn seven_ix_layout() -> VersionedTransaction {
        compiled_tx(&[
            COMPUTE_BUDGET,
            COMPUTE_BUDGET,
            TOKEN_PROGRAM_ID,
            *PHANTOM_LIGHTHOUSE_PROGRAM,
            *PHANTOM_LIGHTHOUSE_PROGRAM,
            *PHANTOM_LIGHTHOUSE_PROGRAM,
            SPL_MEMO_PROGRAM,
        ])
    }

    #[test]
    fn seven_instruction_layout_accepted() {
        let config = SolanaExactFacilitatorConfig::default();
        assert!(validate_instructions(&seven_ix_layout(), &config).is_ok());
    }

    #[test]
    fn eight_instructions_rejected_even_when_max_set_to_8() {
        let config = SolanaExactFacilitatorConfig {
            max_instruction_count: 8,
            ..SolanaExactFacilitatorConfig::default()
        };
        let mut programs = vec![
            COMPUTE_BUDGET,
            COMPUTE_BUDGET,
            TOKEN_PROGRAM_ID,
            *PHANTOM_LIGHTHOUSE_PROGRAM,
            *PHANTOM_LIGHTHOUSE_PROGRAM,
            *PHANTOM_LIGHTHOUSE_PROGRAM,
            SPL_MEMO_PROGRAM,
            *PHANTOM_LIGHTHOUSE_PROGRAM,
        ];
        let tx = compiled_tx(&programs);
        assert!(matches!(
            validate_instructions(&tx, &config),
            Err(SolanaExactError::InstructionCountExceedsMax(7)),
        ));
        programs.pop();
        assert!(validate_instructions(&compiled_tx(&programs), &config).is_ok());
    }

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

    #[tokio::test]
    async fn path2_matches_cpi_inner_transfer_checked() {
        use solana_client::rpc_response::{ParsedInstruction, UiInstruction, UiParsedInstruction};

        use crate::exact::ATA_PROGRAM_PUBKEY;
        use crate::exact::facilitator::smart_wallet::ObservedTransfer;
        use crate::exact::payload::TransactionInt;

        let pay_to = Address::new(Pubkey::new_from_array([1u8; 32]));
        let asset = Address::new(Pubkey::new_from_array([2u8; 32]));
        let dest = {
            let (ata, _) = Pubkey::find_program_address(
                &[pay_to.as_ref(), spl_token::ID.as_ref(), asset.as_ref()],
                &ATA_PROGRAM_PUBKEY,
            );
            ata
        };
        let authority = Pubkey::new_from_array([9u8; 32]);
        let matched = ObservedTransfer {
            program_id: spl_token::ID,
            amount: 1_000,
            mint: *asset.pubkey(),
            destination: dest,
            authority,
        };
        let inner = vec![UiInnerInstructions {
            index: 0,
            instructions: vec![UiInstruction::Parsed(UiParsedInstruction::Parsed(
                ParsedInstruction {
                    program: "spl-token".into(),
                    program_id: spl_token::ID.to_string(),
                    parsed: serde_json::json!({
                        "type": "transferChecked",
                        "info": {
                            "mint": matched.mint.to_string(),
                            "destination": dest.to_string(),
                            "authority": authority.to_string(),
                            "tokenAmount": { "amount": "1000" }
                        }
                    }),
                    stack_height: None,
                },
            ))],
        }];
        let payer = Pubkey::new_from_array([7u8; 32]);
        let wallet = Pubkey::new_from_array([8u8; 32]);
        let tx = compiled_tx(&[wallet]);
        let stub = Path2Stub {
            pubkey: payer,
            inner,
            loaded: solana_client::rpc_response::UiLoadedAddresses::default(),
            smart_wallet: true,
        };
        let config = SolanaExactFacilitatorConfig {
            enable_smart_wallet_verification: true,
            smart_wallet_allowed_programs: Some(vec![Address::new(wallet)]),
            ..SolanaExactFacilitatorConfig::default()
        };
        let requirement = TransferRequirement {
            asset: &asset,
            pay_to: &pay_to,
            amount: 1_000,
        };
        let result = verify_transaction(
            &stub,
            TransactionInt::new(tx).as_base64().unwrap(),
            &requirement,
            &config,
        )
        .await
        .expect("path 2");
        assert_eq!(result.payer, Address::new(authority));
        assert_eq!(result.matched_transfer, Some(matched));
        assert_eq!(
            r402_protocol::AsPaymentProblem::as_payment_problem(
                &super::super::smart_wallet::match_required_transfer(&[], &requirement, &[])
                    .unwrap_err()
            )
            .reason()
            .as_str(),
            "invalid_exact_svm_smart_wallet_no_transfer_in_simulation"
        );
    }

    #[tokio::test]
    async fn path2_unknown_program_is_not_allowed() {
        use crate::exact::payload::TransactionInt;
        let wallet = Pubkey::new_from_array([8u8; 32]);
        let tx = compiled_tx(&[wallet]);
        let stub = Path2Stub {
            pubkey: Pubkey::new_from_array([7u8; 32]),
            inner: Vec::new(),
            loaded: solana_client::rpc_response::UiLoadedAddresses::default(),
            smart_wallet: true,
        };
        let pay_to = Address::new(Pubkey::new_from_array([1u8; 32]));
        let asset = Address::new(Pubkey::new_from_array([2u8; 32]));
        let config = SolanaExactFacilitatorConfig {
            enable_smart_wallet_verification: true,
            ..SolanaExactFacilitatorConfig::default()
        };
        let requirement = TransferRequirement {
            asset: &asset,
            pay_to: &pay_to,
            amount: 1,
        };
        let err = verify_transaction(
            &stub,
            TransactionInt::new(tx).as_base64().unwrap(),
            &requirement,
            &config,
        )
        .await
        .unwrap_err();
        let reason = r402_protocol::AsPaymentProblem::as_payment_problem(&err)
            .reason()
            .as_str()
            .to_owned();
        assert!(reason.contains("invalid_exact_svm_smart_wallet_program_not_allowed"));
        assert_ne!(reason, "invalid_payload");
    }

    #[tokio::test]
    async fn path2_without_provider_caps_is_unavailable() {
        use crate::exact::payload::TransactionInt;
        let wallet = Pubkey::new_from_array([8u8; 32]);
        let tx = compiled_tx(&[wallet]);
        let stub = Path2Stub {
            pubkey: Pubkey::new_from_array([7u8; 32]),
            inner: Vec::new(),
            loaded: solana_client::rpc_response::UiLoadedAddresses::default(),
            smart_wallet: false,
        };
        let pay_to = Address::new(Pubkey::new_from_array([1u8; 32]));
        let asset = Address::new(Pubkey::new_from_array([2u8; 32]));
        let config = SolanaExactFacilitatorConfig {
            enable_smart_wallet_verification: true,
            smart_wallet_allowed_programs: Some(vec![Address::new(wallet)]),
            ..SolanaExactFacilitatorConfig::default()
        };
        let requirement = TransferRequirement {
            asset: &asset,
            pay_to: &pay_to,
            amount: 1,
        };
        let err = verify_transaction(
            &stub,
            TransactionInt::new(tx).as_base64().unwrap(),
            &requirement,
            &config,
        )
        .await
        .unwrap_err();
        assert_eq!(
            r402_protocol::AsPaymentProblem::as_payment_problem(&err)
                .reason()
                .as_str(),
            "invalid_exact_svm_smart_wallet_verification_not_available"
        );
    }

    #[tokio::test]
    async fn path2_matches_compiled_inner_transfer_checked() {
        use solana_client::rpc_response::{UiCompiledInstruction, UiInstruction};

        use crate::exact::ATA_PROGRAM_PUBKEY;
        use crate::exact::payload::TransactionInt;

        let pay_to = Address::new(Pubkey::new_from_array([1u8; 32]));
        let asset = Address::new(Pubkey::new_from_array([2u8; 32]));
        let dest = {
            let (ata, _) = Pubkey::find_program_address(
                &[pay_to.as_ref(), spl_token::ID.as_ref(), asset.as_ref()],
                &ATA_PROGRAM_PUBKEY,
            );
            ata
        };
        let authority = Pubkey::new_from_array([9u8; 32]);
        let source = Pubkey::new_from_array([10u8; 32]);
        let mut data = vec![12u8];
        data.extend_from_slice(&1_000u64.to_le_bytes());
        data.push(6);
        let inner = vec![UiInnerInstructions {
            index: 0,
            instructions: vec![UiInstruction::Compiled(UiCompiledInstruction {
                program_id_index: 6,
                accounts: vec![2, 3, 4, 5],
                data: bs58::encode(data).into_string(),
                stack_height: None,
            })],
        }];
        let loaded = solana_client::rpc_response::UiLoadedAddresses {
            writable: vec![
                source.to_string(),
                asset.pubkey().to_string(),
                dest.to_string(),
                authority.to_string(),
                spl_token::ID.to_string(),
            ],
            readonly: Vec::new(),
        };
        let wallet = Pubkey::new_from_array([8u8; 32]);
        let stub = Path2Stub {
            pubkey: Pubkey::new_from_array([7u8; 32]),
            inner,
            loaded,
            smart_wallet: true,
        };
        let config = SolanaExactFacilitatorConfig {
            enable_smart_wallet_verification: true,
            smart_wallet_allowed_programs: Some(vec![Address::new(wallet)]),
            ..SolanaExactFacilitatorConfig::default()
        };
        let requirement = TransferRequirement {
            asset: &asset,
            pay_to: &pay_to,
            amount: 1_000,
        };
        let result = verify_transaction(
            &stub,
            TransactionInt::new(compiled_tx(&[wallet]))
                .as_base64()
                .unwrap(),
            &requirement,
            &config,
        )
        .await
        .expect("compiled path 2");
        assert_eq!(result.payer, Address::new(authority));
        assert_eq!(result.amount, 1_000);
    }
}
