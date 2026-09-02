//! Shared extra-field parsing for SVM `upto` client, server, and facilitator.

use r402_core::chain::ChainId;
use r402_core::wire::PaymentRequirements;
use serde_json::Value;
use solana_pubkey::Pubkey;

use super::payment_channels::{ChannelSplit, DEFAULT_GRACE_PERIOD_SECONDS};
use super::types::extra_keys;
use crate::chain::{
    Address, SOLANA_NAMESPACE, SolanaChainReference, TOKEN_2022_PROGRAM_ID, TOKEN_PROGRAM_ID,
};
use crate::{CASH, PYUSD, USDC, USDG, USDT};

/// Resolved payment-channel fields derived from SVM `upto` requirements.
#[derive(Debug, Clone)]
pub struct PaymentChannelConfig {
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
/// Returns [`SharedError`] when `feePayer`, `receiverAuthorizer`, or
/// `withdrawDelay` is missing or malformed.
pub fn resolve_payment_channel_config(
    requirements: &PaymentRequirements,
) -> Result<PaymentChannelConfig, SharedError> {
    let extra = requirements.extra.as_ref();
    let fee_payer = extra_string(extra, extra_keys::FEE_PAYER)
        .filter(|s| !s.is_empty())
        .ok_or(SharedError::FeePayer)?;
    let receiver_authorizer = extra_string(extra, extra_keys::RECEIVER_AUTHORIZER)
        .filter(|s| !s.is_empty())
        .ok_or(SharedError::ReceiverAuthorizer)?;
    let withdraw_delay =
        parse_withdraw_delay(extra.and_then(|v| v.get(extra_keys::WITHDRAW_DELAY)))?;
    Ok(PaymentChannelConfig {
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
/// Returns [`SharedError::TokenProgram`] when the hint is present but not a
/// supported SPL token program address.
pub fn parse_token_program_hint(extra: Option<&Value>) -> Result<Option<Pubkey>, SharedError> {
    let Some(raw) = extra.and_then(|v| v.get(extra_keys::TOKEN_PROGRAM)) else {
        return Ok(None);
    };
    if raw.is_null() {
        return Ok(None);
    }
    let Some(hint) = raw.as_str().filter(|s| !s.is_empty()) else {
        return Err(SharedError::TokenProgram(raw.to_string()));
    };
    let program: Pubkey = hint
        .parse()
        .map_err(|_| SharedError::TokenProgram(hint.to_owned()))?;
    if program != TOKEN_PROGRAM_ID && program != TOKEN_2022_PROGRAM_ID {
        return Err(SharedError::TokenProgram(hint.to_owned()));
    }
    Ok(Some(program))
}

/// Resolve the SPL token program owning the requirement's mint.
///
/// # Errors
///
/// Returns [`SharedError::TokenProgram`] when a present hint is invalid.
pub fn resolve_token_program(requirements: &PaymentRequirements) -> Result<Pubkey, SharedError> {
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

fn parse_withdraw_delay(value: Option<&Value>) -> Result<u32, SharedError> {
    let seconds = match value {
        Some(Value::Number(n)) => n
            .as_u64()
            .or_else(|| n.as_i64().and_then(|v| u64::try_from(v).ok()))
            .ok_or(SharedError::WithdrawDelay)?,
        Some(Value::String(s)) => s.parse().map_err(|_| SharedError::WithdrawDelay)?,
        _ => return Err(SharedError::WithdrawDelay),
    };
    let delay = u32::try_from(seconds).map_err(|_| SharedError::WithdrawDelay)?;
    if delay == 0 {
        return Err(SharedError::WithdrawDelay);
    }
    Ok(delay)
}

fn asset_chain(network: &ChainId) -> Result<SolanaChainReference, SharedError> {
    SolanaChainReference::try_from(network.clone()).map_err(|_| SharedError::Network)
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

/// Shared extra-field parse failures.
#[derive(Debug, thiserror::Error)]
pub enum SharedError {
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
