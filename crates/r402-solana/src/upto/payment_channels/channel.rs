//! Onchain channel-account decode and distribution-hash check.
#![allow(
    clippy::indexing_slicing,
    missing_copy_implementations,
    reason = "channel account is rejected unless it is at least CHANNEL_ACCOUNT_SIZE"
)]

use sha2::{Digest, Sha256};
use solana_pubkey::Pubkey;

use super::codec::ChannelSplit;
use super::program::ChannelStatus;

/// Fixed byte length of the channel account layout this scheme targets.
pub const CHANNEL_ACCOUNT_SIZE: usize = 256;

const DISCRIMINATOR: u8 = 1;
const PAYER_OFFSET: usize = 88;
const PAYEE_OFFSET: usize = 120;
const AUTHORIZED_SIGNER_OFFSET: usize = 152;
const MINT_OFFSET: usize = 184;
const RENT_PAYER_OFFSET: usize = 216;
const OPEN_SLOT_OFFSET: usize = 248;

/// Decoded onchain channel account.
#[derive(Debug, Clone)]
pub struct Channel {
    /// Account discriminator (must be 1).
    pub discriminator: u8,
    /// Layout version.
    pub version: u8,
    /// PDA bump.
    pub bump: u8,
    /// Lifecycle status.
    pub status: ChannelStatus,
    /// Derivation salt.
    pub salt: u64,
    /// Escrow deposit.
    pub deposit: u64,
    /// Settled watermark.
    pub settled: u64,
    /// Payout watermark.
    pub payout_watermark: u64,
    /// Forced-close start (unix seconds).
    pub closure_started_at: i64,
    /// Payer withdraw timestamp.
    pub payer_withdrawn_at: i64,
    /// Grace period seconds.
    pub grace_period: u32,
    /// SHA-256 of the sealed distribution.
    pub distribution_hash: [u8; 32],
    /// Channel payer.
    pub payer: Pubkey,
    /// Channel payee.
    pub payee: Pubkey,
    /// Authorized voucher signer.
    pub authorized_signer: Pubkey,
    /// SPL mint.
    pub mint: Pubkey,
    /// Rent payer.
    pub rent_payer: Pubkey,
    /// Open slot.
    pub open_slot: u64,
}

/// Decode a channel account.
///
/// # Errors
///
/// Returns [`DecodeChannelError`] when the buffer is short or the discriminator
/// is not a payment channel.
pub fn decode_channel(data: &[u8]) -> Result<Channel, DecodeChannelError> {
    if data.len() < CHANNEL_ACCOUNT_SIZE {
        return Err(DecodeChannelError::TooShort(data.len()));
    }
    let discriminator = data[0];
    if discriminator != DISCRIMINATOR {
        return Err(DecodeChannelError::NotAChannel(discriminator));
    }
    Ok(Channel {
        discriminator,
        version: data[1],
        bump: data[2],
        status: status_from(data[3]),
        salt: u64_at(data, 4),
        deposit: u64_at(data, 12),
        settled: u64_at(data, 20),
        payout_watermark: u64_at(data, 28),
        closure_started_at: i64_at(data, 36),
        payer_withdrawn_at: i64_at(data, 44),
        grace_period: u32_at(data, 52),
        distribution_hash: data[56..88]
            .try_into()
            .map_err(|_| DecodeChannelError::TooShort(data.len()))?,
        payer: pubkey_at(data, PAYER_OFFSET)?,
        payee: pubkey_at(data, PAYEE_OFFSET)?,
        authorized_signer: pubkey_at(data, AUTHORIZED_SIGNER_OFFSET)?,
        mint: pubkey_at(data, MINT_OFFSET)?,
        rent_payer: pubkey_at(data, RENT_PAYER_OFFSET)?,
        open_slot: u64_at(data, OPEN_SLOT_OFFSET),
    })
}

/// SHA-256 over u32le(count) || each recipient pubkey || u16le(bps).
///
/// # Errors
///
/// Returns [`DecodeChannelError::InvalidRecipient`] when a split address is not
/// valid base58.
pub fn distribution_hash(splits: &[ChannelSplit]) -> Result<[u8; 32], DecodeChannelError> {
    let mut hasher = Sha256::new();
    let count = u32::try_from(splits.len()).map_err(|_| DecodeChannelError::TooManyRecipients)?;
    hasher.update(count.to_le_bytes());
    for split in splits {
        let recipient: Pubkey = split
            .recipient
            .parse()
            .map_err(|_| DecodeChannelError::InvalidRecipient(split.recipient.clone()))?;
        hasher.update(recipient.as_ref());
        hasher.update(split.bps.to_le_bytes());
    }
    Ok(hasher.finalize().into())
}

const fn status_from(raw: u8) -> ChannelStatus {
    match raw {
        1 => ChannelStatus::Closing,
        2 => ChannelStatus::Sealed,
        3 => ChannelStatus::Distributed,
        _ => ChannelStatus::Open,
    }
}

fn u64_at(data: &[u8], offset: usize) -> u64 {
    let mut buf = [0u8; 8];
    buf.copy_from_slice(&data[offset..offset + 8]);
    u64::from_le_bytes(buf)
}

fn i64_at(data: &[u8], offset: usize) -> i64 {
    i64::from_le_bytes(u64_at(data, offset).to_le_bytes())
}

fn u32_at(data: &[u8], offset: usize) -> u32 {
    let mut buf = [0u8; 4];
    buf.copy_from_slice(&data[offset..offset + 4]);
    u32::from_le_bytes(buf)
}

fn pubkey_at(data: &[u8], offset: usize) -> Result<Pubkey, DecodeChannelError> {
    let bytes: [u8; 32] = data
        .get(offset..offset + 32)
        .ok_or(DecodeChannelError::TooShort(data.len()))?
        .try_into()
        .map_err(|_| DecodeChannelError::TooShort(data.len()))?;
    Ok(Pubkey::from(bytes))
}

/// Channel-account decode failures.
#[derive(Debug, thiserror::Error)]
pub enum DecodeChannelError {
    /// Account shorter than the supported layout.
    #[error("channel account is {0} bytes, expected at least {CHANNEL_ACCOUNT_SIZE}")]
    TooShort(usize),
    /// Discriminator is not a payment channel.
    #[error("account discriminator {0} is not a payment channel")]
    NotAChannel(u8),
    /// Recipient is not a valid base58 pubkey.
    #[error("invalid distribution recipient {0}")]
    InvalidRecipient(String),
    /// Recipient count does not fit in u32.
    #[error("too many distribution recipients")]
    TooManyRecipients,
}
