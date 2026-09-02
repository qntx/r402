//! Client-side `open` transaction builder and facilitator-side verifier.
#![allow(
    clippy::indexing_slicing,
    missing_copy_implementations,
    reason = "compiled instruction data is length-checked before field reads"
)]

use solana_compute_budget_interface::ComputeBudgetInstruction;
use solana_instruction::Instruction;
use solana_message::Hash;
use solana_message::v0::Message as MessageV0;
use solana_message::{VersionedMessage, compiled_instruction::CompiledInstruction};
use solana_pubkey::Pubkey;
use solana_signature::Signature;
use solana_transaction::versioned::VersionedTransaction;

use super::codec::{ChannelSplit, CodecError, OpenArgs};
use super::instructions::{OpenInstructionArgs, build_open_instruction};
use super::program::DEFAULT_COMPUTE_UNIT_PRICE_MICROLAMPORTS;
use super::program::{
    OPEN_ACCOUNT_COUNT, OPEN_DEFAULT_COMPUTE_UNIT_LIMIT, OPEN_MAX_COMPUTE_UNIT_LIMIT,
    OPEN_MAX_LIGHTHOUSE_INSTRUCTIONS, OPEN_MAX_OPTIONAL_SUFFIX, OPEN_SLOT_WINDOW,
    PAYMENT_CHANNELS_PROGRAM_ID, RENT_SYSVAR, SYSTEM_PROGRAM_ID, find_ata, find_channel_pda,
    find_event_authority_pda,
};
use crate::SOLANA_MAX_COMPUTE_UNIT_PRICE_MICROLAMPORTS;
use crate::exact::{PHANTOM_LIGHTHOUSE_PROGRAM, SPL_MEMO_PROGRAM};

/// Inputs for the client-side `open` transaction.
#[derive(Debug, Clone)]
pub struct BuildOpenArgs {
    /// Payer (client).
    pub payer: Pubkey,
    /// Channel payee (facilitator fee payer).
    pub payee: Pubkey,
    /// SPL mint.
    pub mint: Pubkey,
    /// Voucher signer.
    pub authorized_signer: Pubkey,
    /// Transaction fee payer and channel rent payer.
    pub fee_payer: Pubkey,
    /// Token program owning the mint.
    pub token_program: Pubkey,
    /// Escrow deposit = authorized ceiling.
    pub deposit: u64,
    /// Recent blockhash for transaction lifetime.
    pub blockhash: Hash,
    /// Slot the open is anchored to.
    pub open_slot: u64,
    /// Forced-close grace period (seconds).
    pub grace_period: u32,
    /// Distribution recipients.
    pub recipients: Vec<ChannelSplit>,
    /// Channel-derivation salt; random when `None`.
    pub salt: Option<u64>,
    /// Seller memo; random hex nonce when `None`.
    pub memo: Option<String>,
}

/// Built (unsigned except for later client signature) open transaction.
#[derive(Debug)]
pub struct BuiltOpen {
    /// Channel PDA.
    pub channel_id: Pubkey,
    /// Versioned transaction (fee-payer signature slot empty).
    pub transaction: VersionedTransaction,
    /// Channel-derivation salt.
    pub salt: u64,
    /// Slot the open is anchored to.
    pub open_slot: u64,
}

/// Build the fee-payer-sponsored `open` transaction.
///
/// # Errors
///
/// Returns [`OpenError`] when args, compute budget, or message compilation fail.
pub fn build_open_transaction(args: BuildOpenArgs) -> Result<BuiltOpen, OpenError> {
    let salt = args.salt.unwrap_or_else(random_u64);
    let (open_ix, channel_id) = build_open_instruction(&OpenInstructionArgs {
        payer: args.payer,
        rent_payer: args.fee_payer,
        payee: args.payee,
        mint: args.mint,
        authorized_signer: args.authorized_signer,
        token_program: args.token_program,
        args: OpenArgs {
            salt,
            deposit: args.deposit,
            grace_period: args.grace_period,
            open_slot: args.open_slot,
            recipients: args.recipients,
        },
    })?;
    let memo_ix = Instruction {
        program_id: SPL_MEMO_PROGRAM,
        accounts: vec![],
        data: resolve_memo_data(args.memo.as_deref())?,
    };
    let mut ixs = open_compute_budget(
        OPEN_DEFAULT_COMPUTE_UNIT_LIMIT,
        DEFAULT_COMPUTE_UNIT_PRICE_MICROLAMPORTS,
    )?;
    ixs.push(open_ix);
    ixs.push(memo_ix);
    let message = MessageV0::try_compile(&args.fee_payer, &ixs, &[], args.blockhash)
        .map_err(|err| OpenError::Compile(err.to_string()))?;
    let num_required = message.header.num_required_signatures as usize;
    Ok(BuiltOpen {
        channel_id,
        transaction: VersionedTransaction {
            signatures: vec![Signature::default(); num_required],
            message: VersionedMessage::V0(message),
        },
        salt,
        open_slot: args.open_slot,
    })
}

fn open_compute_budget(limit: u32, price: u64) -> Result<Vec<Instruction>, OpenError> {
    if limit > OPEN_MAX_COMPUTE_UNIT_LIMIT {
        return Err(OpenError::ComputeUnitLimit(limit));
    }
    if price > SOLANA_MAX_COMPUTE_UNIT_PRICE_MICROLAMPORTS {
        return Err(OpenError::ComputeUnitPrice(price));
    }
    let mut ixs = Vec::new();
    if limit > 0 {
        ixs.push(ComputeBudgetInstruction::set_compute_unit_limit(limit));
    }
    if price > 0 {
        ixs.push(ComputeBudgetInstruction::set_compute_unit_price(price));
    }
    Ok(ixs)
}

fn resolve_memo_data(memo: Option<&str>) -> Result<Vec<u8>, OpenError> {
    const MAX_MEMO_BYTES: usize = 256;
    match memo {
        Some(text) if text.len() > MAX_MEMO_BYTES => Err(OpenError::MemoTooLong(text.len())),
        Some(text) => Ok(text.as_bytes().to_vec()),
        None => Ok(to_hex_bytes(&random_bytes::<16>())),
    }
}

const HEX: &[u8; 16] = b"0123456789abcdef";

fn to_hex_bytes(bytes: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(HEX[usize::from(byte >> 4)]);
        out.push(HEX[usize::from(byte & 0x0f)]);
    }
    out
}

fn random_u64() -> u64 {
    u64::from_le_bytes(random_bytes())
}

fn random_bytes<const N: usize>() -> [u8; N] {
    let mut buf = [0u8; N];
    fill_random(&mut buf);
    buf
}

fn fill_random(buf: &mut [u8]) {
    use rand::RngExt as _;
    rand::rng().fill(buf);
}

/// Pins every value a client-supplied open transaction is checked against.
#[derive(Debug, Clone)]
pub struct VerifyOpenExpected {
    /// Channel authorized signer.
    pub authorized_signer: Pubkey,
    /// Transaction fee payer.
    pub fee_payer: Pubkey,
    /// Payload `from` (payer).
    pub from: Pubkey,
    /// SPL mint.
    pub mint: Pubkey,
    /// Token program.
    pub token_program: Pubkey,
    /// Channel payee.
    pub payee: Pubkey,
    /// Authorized ceiling; deposit must equal this exactly.
    pub max_cap: u64,
    /// `extra.withdrawDelay`.
    pub withdraw_delay: u32,
    /// Payload `openSlot`.
    pub open_slot: u64,
    /// Expected distribution.
    pub recipients: Vec<ChannelSplit>,
    /// Optional freshness window anchor.
    pub recent_slot: Option<u64>,
    /// Required memo data when the seller declared `extra.memo`.
    pub memo: Option<String>,
    /// Operator ceiling for compute units.
    pub max_compute_units: Option<u32>,
    /// Operator ceiling for compute-unit price.
    pub max_priority_fee_micro_lamports: Option<u64>,
    /// Optional required-signer count cap.
    pub max_required_signatures: Option<usize>,
}

/// Channel facts extracted from a verified open.
#[derive(Debug, Clone)]
pub struct VerifyOpenResult {
    /// Channel PDA.
    pub channel_id: Pubkey,
    /// Payer.
    pub payer: Pubkey,
    /// Deposit.
    pub deposit: u64,
    /// Grace period.
    pub grace_period: u32,
    /// Open slot.
    pub open_slot: u64,
    /// Channel salt.
    pub salt: u64,
}

/// Decode and enforce the SVM `upto` open-transaction acceptance policy.
///
/// # Errors
///
/// Returns [`OpenError`] when the transaction fails any static check.
pub fn verify_open_transaction(
    transaction: &VersionedTransaction,
    expected: &VerifyOpenExpected,
) -> Result<VerifyOpenResult, OpenError> {
    let message = &transaction.message;
    if message
        .address_table_lookups()
        .is_some_and(|lookups| !lookups.is_empty())
    {
        return Err(OpenError::AddressLookupTables);
    }
    let open_ix = find_canonical_open_instruction(message, expected)?;
    verify_required_signers(message, expected)?;
    verify_payer_signature(transaction, &expected.from)?;
    verify_open_accounts_and_args(message, &open_ix, expected)
}

fn find_canonical_open_instruction(
    message: &VersionedMessage,
    expected: &VerifyOpenExpected,
) -> Result<CompiledInstruction, OpenError> {
    let instructions = message.instructions();
    let keys = message.static_account_keys();
    let mut index = 0;
    let mut seen_limit = false;
    let mut seen_price = false;
    let max_cu = expected
        .max_compute_units
        .unwrap_or(OPEN_MAX_COMPUTE_UNIT_LIMIT)
        .min(OPEN_MAX_COMPUTE_UNIT_LIMIT);
    let max_price = expected
        .max_priority_fee_micro_lamports
        .unwrap_or(SOLANA_MAX_COMPUTE_UNIT_PRICE_MICROLAMPORTS)
        .min(SOLANA_MAX_COMPUTE_UNIT_PRICE_MICROLAMPORTS);
    while index < instructions.len() {
        let ix = &instructions[index];
        let program = program_of(keys, ix)?;
        if program != solana_compute_budget_interface::ID {
            break;
        }
        reject_fee_payer_in_ix(keys, ix, &expected.fee_payer)?;
        verify_compute_budget_ix(
            &ix.data,
            max_cu,
            max_price,
            &mut seen_limit,
            &mut seen_price,
        )?;
        index += 1;
    }
    let open_ix = instructions
        .get(index)
        .cloned()
        .ok_or(OpenError::MissingOpen)?;
    let open_program = program_of(keys, &open_ix)?;
    if open_program != PAYMENT_CHANNELS_PROGRAM_ID {
        return Err(OpenError::UnexpectedProgram(open_program));
    }
    if open_ix.data.first().copied() != Some(super::program::OPEN_DISCRIMINATOR) {
        return Err(OpenError::NotOpenInstruction);
    }
    verify_optional_suffix(keys, &instructions[index + 1..], expected)?;
    Ok(open_ix)
}

fn verify_optional_suffix(
    keys: &[Pubkey],
    suffix: &[CompiledInstruction],
    expected: &VerifyOpenExpected,
) -> Result<(), OpenError> {
    let mut lighthouse = 0usize;
    let mut memos = Vec::new();
    if suffix.len() > OPEN_MAX_OPTIONAL_SUFFIX {
        return Err(OpenError::OptionalSuffix(suffix.len()));
    }
    for ix in suffix {
        let program = program_of(keys, ix)?;
        if program == *PHANTOM_LIGHTHOUSE_PROGRAM {
            lighthouse += 1;
            if lighthouse > OPEN_MAX_LIGHTHOUSE_INSTRUCTIONS {
                return Err(OpenError::TooManyLighthouse(lighthouse));
            }
            reject_fee_payer_in_ix(keys, ix, &expected.fee_payer)?;
        } else if program == SPL_MEMO_PROGRAM {
            reject_fee_payer_in_ix(keys, ix, &expected.fee_payer)?;
            memos.push(ix.data.clone());
        } else {
            return Err(OpenError::UnexpectedProgram(program));
        }
    }
    if let Some(required) = &expected.memo {
        if memos.len() != 1 {
            return Err(OpenError::MemoCount(memos.len()));
        }
        if memos[0] != required.as_bytes() {
            return Err(OpenError::MemoMismatch);
        }
    }
    Ok(())
}

fn verify_compute_budget_ix(
    data: &[u8],
    max_cu: u32,
    max_price: u64,
    seen_limit: &mut bool,
    seen_price: &mut bool,
) -> Result<(), OpenError> {
    match data.first().copied() {
        Some(2) => {
            if *seen_limit || *seen_price {
                return Err(OpenError::ComputeBudgetOrder);
            }
            if data.len() != 5 {
                return Err(OpenError::MalformedComputeBudget);
            }
            let units = u32::from_le_bytes(
                data[1..5]
                    .try_into()
                    .map_err(|_| OpenError::MalformedComputeBudget)?,
            );
            if units > max_cu {
                return Err(OpenError::ComputeUnitLimit(units));
            }
            *seen_limit = true;
        }
        Some(3) => {
            if *seen_price {
                return Err(OpenError::ComputeBudgetOrder);
            }
            if data.len() != 9 {
                return Err(OpenError::MalformedComputeBudget);
            }
            let price = u64::from_le_bytes(
                data[1..9]
                    .try_into()
                    .map_err(|_| OpenError::MalformedComputeBudget)?,
            );
            if price > max_price {
                return Err(OpenError::ComputeUnitPrice(price));
            }
            *seen_price = true;
        }
        _ => return Err(OpenError::MalformedComputeBudget),
    }
    Ok(())
}

fn verify_required_signers(
    message: &VersionedMessage,
    expected: &VerifyOpenExpected,
) -> Result<(), OpenError> {
    let num = message.header().num_required_signatures as usize;
    if let Some(max) = expected.max_required_signatures
        && num > max
    {
        return Err(OpenError::TooManySigners(num));
    }
    let keys = message.static_account_keys();
    let mut expected_signers = vec![expected.from, expected.fee_payer];
    expected_signers.sort();
    expected_signers.dedup();
    if num != expected_signers.len() {
        return Err(OpenError::SignerCount {
            actual: num,
            expected: expected_signers.len(),
        });
    }
    for key in keys.iter().take(num) {
        if !expected_signers.contains(key) {
            return Err(OpenError::UnexpectedSigner(*key));
        }
    }
    Ok(())
}

fn verify_payer_signature(
    transaction: &VersionedTransaction,
    from: &Pubkey,
) -> Result<(), OpenError> {
    let keys = transaction.message.static_account_keys();
    let index = keys
        .iter()
        .position(|key| key == from)
        .ok_or(OpenError::MissingPayerSignature)?;
    let signature = transaction
        .signatures
        .get(index)
        .ok_or(OpenError::MissingPayerSignature)?;
    if *signature == Signature::default() {
        return Err(OpenError::MissingPayerSignature);
    }
    let msg = transaction.message.serialize();
    if !signature.verify(from.as_ref(), &msg) {
        return Err(OpenError::InvalidPayerSignature);
    }
    Ok(())
}

fn verify_open_accounts_and_args(
    message: &VersionedMessage,
    open_ix: &CompiledInstruction,
    expected: &VerifyOpenExpected,
) -> Result<VerifyOpenResult, OpenError> {
    if open_ix.accounts.len() != OPEN_ACCOUNT_COUNT {
        return Err(OpenError::OpenAccountCount(open_ix.accounts.len()));
    }
    let keys = message.static_account_keys();
    let account = |slot: usize| -> Result<Pubkey, OpenError> {
        let index = usize::from(
            *open_ix
                .accounts
                .get(slot)
                .ok_or(OpenError::MissingAccount(slot))?,
        );
        keys.get(index)
            .copied()
            .ok_or(OpenError::MissingAccount(slot))
    };
    let payer = account(0)?;
    let rent_payer = account(1)?;
    let payee = account(2)?;
    let mint = account(3)?;
    let authorized_signer = account(4)?;
    let channel = account(5)?;
    let payer_ata = account(6)?;
    let channel_ata = account(7)?;
    let token_program = account(8)?;
    verify_open_privileges(
        message,
        open_ix,
        &[payer, rent_payer, channel, payer_ata, channel_ata],
    )?;
    if payer != expected.from
        || keys.first().copied() != Some(expected.fee_payer)
        || rent_payer != expected.fee_payer
        || payee != expected.payee
        || mint != expected.mint
        || authorized_signer != expected.authorized_signer
        || token_program != expected.token_program
    {
        return Err(OpenError::AccountMismatch);
    }
    let (want_payer_ata, _) = find_ata(&payer, &expected.mint, &expected.token_program);
    let (want_channel_ata, _) = find_ata(&channel, &expected.mint, &expected.token_program);
    let (event_authority, _) = find_event_authority_pda();
    if payer_ata != want_payer_ata
        || channel_ata != want_channel_ata
        || account(9)? != SYSTEM_PROGRAM_ID
        || account(10)? != RENT_SYSVAR
        || account(11)? != super::program::ASSOCIATED_TOKEN_PROGRAM_ID
        || account(12)? != event_authority
        || account(13)? != PAYMENT_CHANNELS_PROGRAM_ID
    {
        return Err(OpenError::AccountMismatch);
    }
    let args = super::codec::decode_open_args(open_ix.data.get(1..).unwrap_or(&[]))?;
    verify_open_args(&args, expected)?;
    let (derived, _) = find_channel_pda(
        &payer,
        &payee,
        &mint,
        &authorized_signer,
        args.salt,
        args.open_slot,
    );
    if derived != channel {
        return Err(OpenError::ChannelPdaMismatch);
    }
    Ok(VerifyOpenResult {
        channel_id: channel,
        payer,
        deposit: args.deposit,
        grace_period: args.grace_period,
        open_slot: args.open_slot,
        salt: args.salt,
    })
}

fn verify_open_privileges(
    message: &VersionedMessage,
    open_ix: &CompiledInstruction,
    writable_roles: &[Pubkey],
) -> Result<(), OpenError> {
    let keys = message.static_account_keys();
    let header = message.header();
    let signer_count = header.num_required_signatures as usize;
    let payer_idx = usize::from(
        *open_ix
            .accounts
            .first()
            .ok_or(OpenError::MissingAccount(0))?,
    );
    let rent_idx = usize::from(
        *open_ix
            .accounts
            .get(1)
            .ok_or(OpenError::MissingAccount(1))?,
    );
    if payer_idx >= signer_count || rent_idx >= signer_count {
        return Err(OpenError::MissingSignerRole);
    }
    for (index, key) in keys.iter().enumerate() {
        if is_writable_index(*header, keys.len(), index) && !writable_roles.contains(key) {
            return Err(OpenError::UnexpectedWritable(*key));
        }
    }
    Ok(())
}

fn verify_open_args(args: &OpenArgs, expected: &VerifyOpenExpected) -> Result<(), OpenError> {
    if args.deposit == 0 || args.deposit != expected.max_cap {
        return Err(OpenError::DepositMismatch {
            actual: args.deposit,
            expected: expected.max_cap,
        });
    }
    if args.grace_period != expected.withdraw_delay {
        return Err(OpenError::GracePeriodMismatch);
    }
    if args.open_slot != expected.open_slot {
        return Err(OpenError::OpenSlotMismatch);
    }
    if expected.recent_slot.is_some_and(|recent| {
        args.open_slot > recent || recent.saturating_sub(args.open_slot) > OPEN_SLOT_WINDOW
    }) {
        return Err(OpenError::OpenSlotWindow);
    }
    if args.recipients != expected.recipients {
        return Err(OpenError::RecipientsMismatch);
    }
    Ok(())
}

fn program_of(keys: &[Pubkey], ix: &CompiledInstruction) -> Result<Pubkey, OpenError> {
    keys.get(ix.program_id_index as usize)
        .copied()
        .ok_or(OpenError::MissingAccount(ix.program_id_index as usize))
}

fn reject_fee_payer_in_ix(
    keys: &[Pubkey],
    ix: &CompiledInstruction,
    fee_payer: &Pubkey,
) -> Result<(), OpenError> {
    if program_of(keys, ix)? == *fee_payer {
        return Err(OpenError::FeePayerInInstruction);
    }
    for index in &ix.accounts {
        if keys.get(usize::from(*index)) == Some(fee_payer) {
            return Err(OpenError::FeePayerInInstruction);
        }
    }
    Ok(())
}

const fn is_writable_index(
    header: solana_message::MessageHeader,
    account_keys: usize,
    index: usize,
) -> bool {
    let num_required = header.num_required_signatures as usize;
    let num_readonly_signed = header.num_readonly_signed_accounts as usize;
    let num_readonly_unsigned = header.num_readonly_unsigned_accounts as usize;
    if index < num_required {
        index < num_required.saturating_sub(num_readonly_signed)
    } else {
        index < account_keys.saturating_sub(num_readonly_unsigned)
    }
}

/// Open builder / verifier failures.
#[derive(Debug, thiserror::Error)]
pub enum OpenError {
    /// Open-args codec failure.
    #[error(transparent)]
    Codec(#[from] CodecError),
    /// Message compilation failed.
    #[error("failed to compile open transaction: {0}")]
    Compile(String),
    /// Compute unit limit out of range.
    #[error("computeUnitLimit {0} exceeds the spec ceiling")]
    ComputeUnitLimit(u32),
    /// Compute unit price out of range.
    #[error("computeUnitPrice {0} exceeds the spec ceiling")]
    ComputeUnitPrice(u64),
    /// Seller memo exceeds 256 bytes.
    #[error("extra.memo exceeds maximum 256 bytes ({0})")]
    MemoTooLong(usize),
    /// Address lookup tables hide accounts from static verification.
    #[error("address lookup tables are not permitted in an open transaction")]
    AddressLookupTables,
    /// No payment-channels `open` instruction.
    #[error("no payment-channels open instruction found")]
    MissingOpen,
    /// Instruction program is not payment-channels / memo / lighthouse.
    #[error("unexpected instruction program {0}")]
    UnexpectedProgram(Pubkey),
    /// Payment-channels instruction is not `open`.
    #[error("payment-channels instruction is not open")]
    NotOpenInstruction,
    /// Too many optional suffix instructions.
    #[error(
        "at most {OPEN_MAX_OPTIONAL_SUFFIX} optional instructions are allowed after open (found {0})"
    )]
    OptionalSuffix(usize),
    /// Too many Lighthouse instructions.
    #[error(
        "at most {OPEN_MAX_LIGHTHOUSE_INSTRUCTIONS} Lighthouse instructions are allowed after open (found {0})"
    )]
    TooManyLighthouse(usize),
    /// Memo instruction count invalid when `extra.memo` is set.
    #[error("expected exactly one Memo instruction matching extra.memo, found {0}")]
    MemoCount(usize),
    /// Memo data mismatch.
    #[error("Memo instruction data does not match extra.memo")]
    MemoMismatch,
    /// Compute-budget instruction order / duplicate.
    #[error("invalid ComputeBudget instruction order")]
    ComputeBudgetOrder,
    /// Malformed compute-budget instruction.
    #[error("malformed ComputeBudget instruction")]
    MalformedComputeBudget,
    /// Required-signer count exceeds operator cap.
    #[error("required-signer count {0} exceeds maxRequiredSignatures")]
    TooManySigners(usize),
    /// Required-signer count does not match {{from, feePayer}}.
    #[error("required-signer count {actual} != expected {expected}")]
    SignerCount {
        /// Observed signer count.
        actual: usize,
        /// Expected distinct {{from, feePayer}} count.
        expected: usize,
    },
    /// Unexpected required signer.
    #[error("unexpected required signer {0}")]
    UnexpectedSigner(Pubkey),
    /// Payer signature missing or default.
    #[error("missing signature for payload.from")]
    MissingPayerSignature,
    /// Payer signature does not verify.
    #[error("invalid signature for payload.from")]
    InvalidPayerSignature,
    /// Open instruction account count is not 14.
    #[error("open instruction must have exactly {OPEN_ACCOUNT_COUNT} accounts, found {0}")]
    OpenAccountCount(usize),
    /// Account index out of range.
    #[error("missing account at slot {0}")]
    MissingAccount(usize),
    /// Payer / rent payer / mint / ATA / program accounts do not match.
    #[error("open instruction accounts do not match the payment requirements")]
    AccountMismatch,
    /// Channel PDA does not match derived seeds.
    #[error("channel PDA does not match derived address")]
    ChannelPdaMismatch,
    /// Payer or rent payer is not a required signer.
    #[error("payer and rentPayer must be signers")]
    MissingSignerRole,
    /// Writable account outside the open writable roles.
    #[error("account {0} is writable but is not among the open instruction's writable roles")]
    UnexpectedWritable(Pubkey),
    /// Deposit is zero or does not equal the ceiling.
    #[error("deposit {actual} != maxCap {expected}")]
    DepositMismatch {
        /// Encoded deposit.
        actual: u64,
        /// Authorized ceiling.
        expected: u64,
    },
    /// Grace period does not match `extra.withdrawDelay`.
    #[error("gracePeriod does not match extra.withdrawDelay")]
    GracePeriodMismatch,
    /// Open slot does not match payload.
    #[error("openSlot does not match payload.openSlot")]
    OpenSlotMismatch,
    /// Open slot is outside the freshness window.
    #[error("openSlot is outside the {OPEN_SLOT_WINDOW}-slot freshness window")]
    OpenSlotWindow,
    /// Distribution recipients do not match `payTo` 100%.
    #[error("distribution recipients do not match the payment requirements")]
    RecipientsMismatch,
    /// Fee payer appears in a compute-budget, memo, or lighthouse instruction.
    #[error("feePayer must not appear in non-open instruction accounts")]
    FeePayerInInstruction,
}
