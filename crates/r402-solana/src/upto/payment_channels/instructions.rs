//! Instruction builders for `open`, `settle_and_seal`, `distribute`, `reclaim`.
#![allow(
    clippy::indexing_slicing,
    reason = "Ed25519 precompile header is a fixed 16-byte layout"
)]

use solana_instruction::{AccountMeta, Instruction};
use solana_pubkey::Pubkey;

use super::codec::{
    ChannelSplit, CodecError, OpenArgs, encode_distribution_entries, encode_open_args,
};
use super::program::{
    ASSOCIATED_TOKEN_PROGRAM_ID, DISTRIBUTE_DISCRIMINATOR, ED25519_PROGRAM_ID, INSTRUCTIONS_SYSVAR,
    OPEN_DISCRIMINATOR, PAYMENT_CHANNELS_PROGRAM_ID, RECLAIM_DISCRIMINATOR, RENT_SYSVAR,
    SETTLE_AND_SEAL_DISCRIMINATOR, SYSTEM_PROGRAM_ID, find_ata, find_channel_pda,
    find_event_authority_pda, treasury_owner,
};
use super::voucher::VOUCHER_MESSAGE_SIZE;

/// Accounts and data for a canonical 14-account `open`.
#[derive(Debug, Clone)]
pub struct OpenInstructionArgs {
    /// Channel payer (client).
    pub payer: Pubkey,
    /// Channel rent payer (facilitator fee payer).
    pub rent_payer: Pubkey,
    /// Channel payee (facilitator fee payer, zero share).
    pub payee: Pubkey,
    /// SPL mint.
    pub mint: Pubkey,
    /// Voucher signer recorded in the channel.
    pub authorized_signer: Pubkey,
    /// Token program owning the mint.
    pub token_program: Pubkey,
    /// Encoded open arguments.
    pub args: OpenArgs,
}

/// Build the canonical 14-account `open` instruction.
///
/// # Errors
///
/// Returns [`CodecError`] when open args cannot be encoded.
pub fn build_open_instruction(
    args: &OpenInstructionArgs,
) -> Result<(Instruction, Pubkey), CodecError> {
    let (channel, _) = find_channel_pda(
        &args.payer,
        &args.payee,
        &args.mint,
        &args.authorized_signer,
        args.args.salt,
        args.args.open_slot,
    );
    let (payer_ata, _) = find_ata(&args.payer, &args.mint, &args.token_program);
    let (channel_ata, _) = find_ata(&channel, &args.mint, &args.token_program);
    let (event_authority, _) = find_event_authority_pda();
    let encoded = encode_open_args(&args.args)?;
    let mut data = Vec::with_capacity(1 + encoded.len());
    data.push(OPEN_DISCRIMINATOR);
    data.extend_from_slice(&encoded);
    let accounts = vec![
        AccountMeta::new(args.payer, true),
        AccountMeta::new(args.rent_payer, true),
        AccountMeta::new_readonly(args.payee, false),
        AccountMeta::new_readonly(args.mint, false),
        AccountMeta::new_readonly(args.authorized_signer, false),
        AccountMeta::new(channel, false),
        AccountMeta::new(payer_ata, false),
        AccountMeta::new(channel_ata, false),
        AccountMeta::new_readonly(args.token_program, false),
        AccountMeta::new_readonly(SYSTEM_PROGRAM_ID, false),
        AccountMeta::new_readonly(RENT_SYSVAR, false),
        AccountMeta::new_readonly(ASSOCIATED_TOKEN_PROGRAM_ID, false),
        AccountMeta::new_readonly(event_authority, false),
        AccountMeta::new_readonly(PAYMENT_CHANNELS_PROGRAM_ID, false),
    ];
    Ok((
        Instruction {
            program_id: PAYMENT_CHANNELS_PROGRAM_ID,
            accounts,
            data,
        },
        channel,
    ))
}

/// Build the payee-signed cooperative close.
///
/// When `has_voucher` is true, an Ed25519 precompile instruction carrying the
/// voucher must immediately precede this instruction.
#[must_use]
pub fn build_settle_and_seal_instruction(
    channel: Pubkey,
    payee: Pubkey,
    has_voucher: bool,
) -> Instruction {
    Instruction {
        program_id: PAYMENT_CHANNELS_PROGRAM_ID,
        accounts: vec![
            AccountMeta::new_readonly(payee, true),
            AccountMeta::new(channel, false),
            AccountMeta::new_readonly(INSTRUCTIONS_SYSVAR, false),
        ],
        data: vec![SETTLE_AND_SEAL_DISCRIMINATOR, u8::from(has_voucher)],
    }
}

/// Accounts for `distribute`.
#[derive(Debug, Clone)]
pub struct DistributeInstructionArgs {
    /// Channel PDA.
    pub channel: Pubkey,
    /// Channel payer (client).
    pub payer: Pubkey,
    /// Channel payee (facilitator).
    pub payee: Pubkey,
    /// Recorded rent payer.
    pub rent_payer: Pubkey,
    /// SPL mint.
    pub mint: Pubkey,
    /// Token program owning the mint.
    pub token_program: Pubkey,
    /// Distribution recipients.
    pub splits: Vec<ChannelSplit>,
    /// CAIP-2 network selecting the treasury owner.
    pub network: String,
}

/// Build `distribute` with a writable recipient token account per split.
///
/// # Errors
///
/// Returns [`CodecError`] when a recipient address is invalid.
pub fn build_distribute_instruction(
    args: &DistributeInstructionArgs,
) -> Result<Instruction, CodecError> {
    let (channel_ata, _) = find_ata(&args.channel, &args.mint, &args.token_program);
    let (payer_ata, _) = find_ata(&args.payer, &args.mint, &args.token_program);
    let (payee_ata, _) = find_ata(&args.payee, &args.mint, &args.token_program);
    let (treasury_ata, _) = find_ata(
        &treasury_owner(&args.network),
        &args.mint,
        &args.token_program,
    );
    let (event_authority, _) = find_event_authority_pda();
    let mut accounts = vec![
        AccountMeta::new(args.channel, false),
        AccountMeta::new(args.payer, false),
        AccountMeta::new(args.rent_payer, false),
        AccountMeta::new(channel_ata, false),
        AccountMeta::new(payer_ata, false),
        AccountMeta::new(payee_ata, false),
        AccountMeta::new(treasury_ata, false),
        AccountMeta::new_readonly(args.mint, false),
        AccountMeta::new_readonly(args.token_program, false),
        AccountMeta::new_readonly(event_authority, false),
        AccountMeta::new_readonly(PAYMENT_CHANNELS_PROGRAM_ID, false),
    ];
    for split in &args.splits {
        let recipient: Pubkey = split
            .recipient
            .parse()
            .map_err(|_| CodecError::InvalidRecipient(split.recipient.clone()))?;
        let (ata, _) = find_ata(&recipient, &args.mint, &args.token_program);
        accounts.push(AccountMeta::new(ata, false));
    }
    let entries = encode_distribution_entries(&args.splits)?;
    let mut data = Vec::with_capacity(1 + entries.len());
    data.push(DISTRIBUTE_DISCRIMINATOR);
    data.extend_from_slice(&entries);
    Ok(Instruction {
        program_id: PAYMENT_CHANNELS_PROGRAM_ID,
        accounts,
        data,
    })
}

/// Permissionless rent reclaim for a Distributed channel.
#[must_use]
pub fn build_reclaim_instruction(channel: Pubkey, rent_payer: Pubkey) -> Instruction {
    Instruction {
        program_id: PAYMENT_CHANNELS_PROGRAM_ID,
        accounts: vec![
            AccountMeta::new(channel, false),
            AccountMeta::new(rent_payer, false),
        ],
        data: vec![RECLAIM_DISCRIMINATOR],
    }
}

/// Ed25519 precompile instruction over the 50-byte voucher message.
#[must_use]
pub fn build_ed25519_verify_instruction(
    message: &[u8; VOUCHER_MESSAGE_SIZE],
    signature: &[u8; 64],
    signer: &Pubkey,
) -> Instruction {
    const PUBLIC_KEY_OFFSET: u16 = 16;
    const SIGNATURE_OFFSET: u16 = PUBLIC_KEY_OFFSET + 32;
    const MESSAGE_DATA_OFFSET: u16 = SIGNATURE_OFFSET + 64;
    const CURRENT_INSTRUCTION: u16 = 0xffff;
    let mut data = vec![0u8; usize::from(MESSAGE_DATA_OFFSET) + message.len()];
    data[0] = 1;
    data[1] = 0;
    data[2..4].copy_from_slice(&SIGNATURE_OFFSET.to_le_bytes());
    data[4..6].copy_from_slice(&CURRENT_INSTRUCTION.to_le_bytes());
    data[6..8].copy_from_slice(&PUBLIC_KEY_OFFSET.to_le_bytes());
    data[8..10].copy_from_slice(&CURRENT_INSTRUCTION.to_le_bytes());
    data[10..12].copy_from_slice(&MESSAGE_DATA_OFFSET.to_le_bytes());
    data[12..14].copy_from_slice(&u16::try_from(message.len()).unwrap_or(0).to_le_bytes());
    data[14..16].copy_from_slice(&CURRENT_INSTRUCTION.to_le_bytes());
    let pk_off = usize::from(PUBLIC_KEY_OFFSET);
    data[pk_off..pk_off + 32].copy_from_slice(signer.as_ref());
    let sig_off = usize::from(SIGNATURE_OFFSET);
    data[sig_off..sig_off + 64].copy_from_slice(signature);
    let msg_off = usize::from(MESSAGE_DATA_OFFSET);
    data[msg_off..msg_off + message.len()].copy_from_slice(message);
    Instruction {
        program_id: ED25519_PROGRAM_ID,
        accounts: vec![],
        data,
    }
}
