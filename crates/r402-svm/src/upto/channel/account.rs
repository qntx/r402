//! Channel-account layout decode and extra-field channel config.

use r402_protocol::{ChainId, PaymentRequirements};
use serde_json::Value;
use solana_pubkey::Pubkey;

use super::codec::ChannelSplit;
use super::program::DEFAULT_GRACE_PERIOD_SECONDS;
use crate::chain::{
    Address, SOLANA_NAMESPACE, SolanaChainReference, TOKEN_2022_PROGRAM_ID, TOKEN_PROGRAM_ID,
};
use crate::upto::payload::extra_keys;
use crate::{CASH, PYUSD, USDC, USDG, USDT};

/// Resolved payment-channel fields derived from SVM `upto` requirements.
#[derive(Debug, Clone)]
pub struct ChannelConfig {
    /// Transaction fee payer, rent payer, and zero-share channel payee.
    pub fee_payer: String,
    /// Authorized voucher signer (server hot key).
    pub receiver_authorizer: String,
    /// Forced-close grace period in seconds.
    pub withdraw_delay: u32,
    /// Program distribution recipients (always 100% to `payTo`).
    pub splits: Vec<ChannelSplit>,
}

/// Resolve and validate SVM `upto` payment-channel fields.
///
/// # Errors
///
/// Returns [`ChannelConfigError`] when `feePayer`, `receiverAuthorizer`, or
/// `withdrawDelay` is missing or malformed.
pub fn resolve_payment_channel_config(
    requirements: &PaymentRequirements,
) -> Result<ChannelConfig, ChannelConfigError> {
    let extra = requirements.extra.as_ref();
    let fee_payer = extra_string(extra, extra_keys::FEE_PAYER)
        .filter(|s| !s.is_empty())
        .ok_or(ChannelConfigError::FeePayer)?;
    let receiver_authorizer = extra_string(extra, extra_keys::RECEIVER_AUTHORIZER)
        .filter(|s| !s.is_empty())
        .ok_or(ChannelConfigError::ReceiverAuthorizer)?;
    let withdraw_delay =
        parse_withdraw_delay(extra.and_then(|v| v.get(extra_keys::WITHDRAW_DELAY)))?;
    Ok(ChannelConfig {
        fee_payer: fee_payer.to_owned(),
        receiver_authorizer: receiver_authorizer.to_owned(),
        withdraw_delay,
        splits: vec![ChannelSplit::full(requirements.pay_to.as_str())],
    })
}

/// Read and validate `extra.tokenProgram`.
///
/// # Errors
///
/// Returns [`ChannelConfigError::TokenProgram`] when the hint is present but
/// not a supported SPL token program address.
pub fn parse_token_program_hint(
    extra: Option<&Value>,
) -> Result<Option<Pubkey>, ChannelConfigError> {
    let Some(raw) = extra.and_then(|v| v.get(extra_keys::TOKEN_PROGRAM)) else {
        return Ok(None);
    };
    if raw.is_null() {
        return Ok(None);
    }
    let Some(hint) = raw.as_str().filter(|s| !s.is_empty()) else {
        return Err(ChannelConfigError::TokenProgram(raw.to_string()));
    };
    let program: Pubkey = hint
        .parse()
        .map_err(|_| ChannelConfigError::TokenProgram(hint.to_owned()))?;
    if program != TOKEN_PROGRAM_ID && program != TOKEN_2022_PROGRAM_ID {
        return Err(ChannelConfigError::TokenProgram(hint.to_owned()));
    }
    Ok(Some(program))
}

/// Resolve the SPL token program owning the requirement's mint.
///
/// # Errors
///
/// Returns [`ChannelConfigError::TokenProgram`] when a present hint is invalid.
pub fn resolve_token_program(
    requirements: &PaymentRequirements,
) -> Result<Pubkey, ChannelConfigError> {
    if let Some(hint) = parse_token_program_hint(requirements.extra.as_ref())? {
        return Ok(hint);
    }
    Ok(stablecoin_token_program(
        requirements.asset.as_str(),
        &requirements.network,
    ))
}

/// Registry token program for a known stablecoin mint; otherwise SPL Token.
#[must_use]
pub fn stablecoin_token_program(asset: &str, network: &ChainId) -> Pubkey {
    if network.namespace() != SOLANA_NAMESPACE {
        return TOKEN_PROGRAM_ID;
    }
    let Ok(chain) = asset_chain(network) else {
        return TOKEN_PROGRAM_ID;
    };
    let Ok(mint) = asset.parse::<Address>() else {
        return TOKEN_PROGRAM_ID;
    };
    default_mints()
        .find(|(deployment, _)| deployment.chain_reference == chain && deployment.address == mint)
        .map_or(TOKEN_PROGRAM_ID, |(deployment, _)| deployment.token_program)
}

/// Optional seller memo (`extra.memo`). Empty / non-string is unset.
#[must_use]
pub fn parse_extra_memo(extra: Option<&Value>) -> Option<String> {
    extra_string(extra, extra_keys::MEMO)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
}

/// Optional decimal u64 hint (`recentSlot`, `lastValidBlockHeight`, `validAfter`).
#[must_use]
pub fn parse_extra_u64(value: Option<&Value>) -> Option<u64> {
    match value? {
        Value::Number(n) => n
            .as_u64()
            .or_else(|| n.as_i64().and_then(|v| u64::try_from(v).ok())),
        Value::String(s) => s.parse().ok(),
        _ => None,
    }
}

/// Default grace period: `max(900, max_timeout_seconds)`.
#[must_use]
pub fn default_withdraw_delay(max_timeout_seconds: u64) -> u32 {
    let timeout = u32::try_from(max_timeout_seconds).unwrap_or(u32::MAX);
    DEFAULT_GRACE_PERIOD_SECONDS.max(timeout)
}

fn extra_string<'a>(extra: Option<&'a Value>, key: &str) -> Option<&'a str> {
    extra.and_then(|v| v.get(key)).and_then(Value::as_str)
}

fn parse_withdraw_delay(value: Option<&Value>) -> Result<u32, ChannelConfigError> {
    let seconds = match value {
        Some(Value::Number(n)) => n
            .as_u64()
            .or_else(|| n.as_i64().and_then(|v| u64::try_from(v).ok()))
            .ok_or(ChannelConfigError::WithdrawDelay)?,
        Some(Value::String(s)) => s.parse().map_err(|_| ChannelConfigError::WithdrawDelay)?,
        _ => return Err(ChannelConfigError::WithdrawDelay),
    };
    let delay = u32::try_from(seconds).map_err(|_| ChannelConfigError::WithdrawDelay)?;
    if delay == 0 {
        return Err(ChannelConfigError::WithdrawDelay);
    }
    Ok(delay)
}

fn asset_chain(network: &ChainId) -> Result<SolanaChainReference, ChannelConfigError> {
    SolanaChainReference::try_from(network.clone()).map_err(|_| ChannelConfigError::Network)
}

fn default_mints()
-> impl Iterator<Item = (&'static crate::chain::SolanaTokenDeployment, &'static str)> {
    USDC::all()
        .iter()
        .map(|d| (d, "USDC"))
        .chain(USDT::all().iter().map(|d| (d, "USDT")))
        .chain(USDG::all().iter().map(|d| (d, "USDG")))
        .chain(PYUSD::all().iter().map(|d| (d, "PYUSD")))
        .chain(CASH::all().iter().map(|d| (d, "CASH")))
}

/// Extra-field parse failures for channel config.
#[derive(Debug, thiserror::Error)]
pub enum ChannelConfigError {
    /// `feePayer` missing or empty.
    #[error("feePayer must be a non-empty string")]
    FeePayer,
    /// `receiverAuthorizer` missing or empty.
    #[error("receiverAuthorizer must be a non-empty string")]
    ReceiverAuthorizer,
    /// `withdrawDelay` missing or not a positive integer.
    #[error("withdrawDelay must be an integer greater than zero")]
    WithdrawDelay,
    /// `tokenProgram` is not a supported SPL token program.
    #[error("tokenProgram {0} is not a supported SPL token program")]
    TokenProgram(String),
    /// Network is not a Solana CAIP-2 id.
    #[error("unsupported Solana network")]
    Network,
}

#[cfg(feature = "facilitator")]
mod onchain {
    #![allow(
        clippy::indexing_slicing,
        missing_copy_implementations,
        reason = "channel account is rejected unless it is at least CHANNEL_ACCOUNT_SIZE"
    )]

    use sha2::{Digest, Sha256};
    use solana_pubkey::Pubkey;

    use super::super::codec::ChannelSplit;
    use super::super::program::ChannelStatus;

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
    /// Returns [`DecodeChannelError`] when the buffer is short or the
    /// discriminator is not a payment channel.
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
    /// Returns [`DecodeChannelError::InvalidRecipient`] when a split address
    /// is not valid base58.
    pub fn distribution_hash(splits: &[ChannelSplit]) -> Result<[u8; 32], DecodeChannelError> {
        let mut hasher = Sha256::new();
        let count =
            u32::try_from(splits.len()).map_err(|_| DecodeChannelError::TooManyRecipients)?;
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
}

#[cfg(feature = "facilitator")]
pub use onchain::{
    CHANNEL_ACCOUNT_SIZE, Channel, DecodeChannelError, decode_channel, distribution_hash,
};
