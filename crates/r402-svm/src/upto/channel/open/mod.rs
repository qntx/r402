//! Client-side `open` transaction builder and facilitator-side verifier.
#![allow(
    clippy::indexing_slicing,
    missing_copy_implementations,
    reason = "compiled instruction data is length-checked before field reads"
)]

use solana_compute_budget_interface::ComputeBudgetInstruction;
use solana_message::v0::Message as MessageV0;
use solana_message::{Hash, VersionedMessage};
use solana_pubkey::Pubkey;
use solana_signature::Signature;
use solana_transaction::Instruction;
use solana_transaction::versioned::VersionedTransaction;

use super::codec::{ChannelSplit, CodecError, OpenArgs};
use super::instructions::{OpenInstructionArgs, build_open_instruction};
use super::program::{
    DEFAULT_COMPUTE_UNIT_PRICE_MICROLAMPORTS, OPEN_ACCOUNT_COUNT, OPEN_DEFAULT_COMPUTE_UNIT_LIMIT,
    OPEN_MAX_COMPUTE_UNIT_LIMIT, OPEN_MAX_LIGHTHOUSE_INSTRUCTIONS, OPEN_MAX_OPTIONAL_SUFFIX,
    OPEN_SLOT_WINDOW,
};
use crate::chain::SOLANA_MAX_COMPUTE_UNIT_PRICE_MICROLAMPORTS;
use crate::exact::SPL_MEMO_PROGRAM;

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
    rand::fill(buf);
}

mod accounts;
pub use accounts::{VerifyOpenExpected, VerifyOpenResult, verify_open_transaction};

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
