//! Read-only open-authorization checks shared by verify and deposit settle.
#![allow(
    unreachable_pub,
    clippy::too_many_lines,
    reason = "shared by verify and settle in this crate"
)]

use std::str::FromStr;
use std::time::{SystemTime, UNIX_EPOCH};

use r402_protocol::{
    Base64Bytes, FacilitatorError, SettleResponse, VerificationError, VerifyResponse,
};
use solana_pubkey::Pubkey;
use solana_transaction::versioned::VersionedTransaction;

use super::config::{EXPIRES_AT_CLOCK_SKEW_SECS, SolanaUptoFacilitatorConfig};
use crate::chain::provider::SolanaChainProviderLike;
use crate::upto::channel::{
    ChannelConfig, VerifyOpenExpected, parse_extra_memo, resolve_payment_channel_config,
    resolve_token_program, verify_open_transaction,
};
use crate::upto::error::{codes, upto_reason};
use crate::upto::payload::{UptoScheme, UptoSvmPayload};

/// Validated open-authorization context.
pub struct OpenAuth {
    /// Typed payload.
    pub payload: UptoSvmPayload,
    /// Channel config from requirements.
    #[allow(dead_code, reason = "retained for claim/deposit callers")]
    pub(crate) channel: ChannelConfig,
    /// Authorized ceiling.
    pub max_amount: u64,
    /// Token program.
    pub token_program: Pubkey,
    /// Decoded open transaction.
    pub transaction: VersionedTransaction,
}

/// Structured SVM `upto` verify/settle failure.
#[derive(Debug)]
pub struct UptoFailure {
    /// Wire error code.
    pub code: &'static str,
    /// Payer address if known.
    pub payer: String,
    /// Human-readable detail.
    pub message: String,
}

impl UptoFailure {
    /// Construct a failure.
    #[must_use]
    pub fn new(code: &'static str, payer: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code,
            payer: payer.into(),
            message: message.into(),
        }
    }

    /// Wire verify response.
    #[must_use]
    pub fn into_verify(self) -> VerifyResponse {
        VerifyResponse::invalid_with_message(
            Some(self.payer.into()),
            upto_reason(self.code),
            self.message,
        )
    }

    /// Wire settle response.
    #[must_use]
    pub fn into_settle(self, network: impl Into<compact_str::CompactString>) -> SettleResponse {
        SettleResponse::Failure {
            reason: upto_reason(self.code),
            message: Some(self.message.into()),
            payer: Some(self.payer.into()),
            transaction: compact_str::CompactString::default(),
            network: network.into(),
            extensions: r402_protocol::Extensions::new(),
            extension_responses: r402_protocol::Extensions::new(),
            extra: None,
        }
    }

    /// Facilitator error for `Err` paths that cannot return a settle body.
    #[must_use]
    pub fn into_error(self) -> FacilitatorError {
        FacilitatorError::Verification(VerificationError::InvalidFormat(format!(
            "{}: {}",
            self.code, self.message
        )))
    }
}

/// Static open-authorization checks. Never broadcasts.
pub fn validate_open_authorization<P: SolanaChainProviderLike>(
    provider: &P,
    payload: &UptoSvmPayload,
    requirements: &r402_protocol::PaymentRequirements,
    accepted_scheme: &str,
    accepted_network: &str,
    reject_voucher: bool,
    config: &SolanaUptoFacilitatorConfig,
) -> Result<OpenAuth, UptoFailure> {
    if accepted_scheme != UptoScheme::VALUE || requirements.scheme.as_str() != UptoScheme::VALUE {
        return Err(UptoFailure::new(
            "unsupported_scheme",
            &payload.from,
            "unsupported_scheme",
        ));
    }
    if accepted_network != requirements.network.to_string() {
        return Err(UptoFailure::new(
            "network_mismatch",
            &payload.from,
            "network_mismatch",
        ));
    }
    if reject_voucher && payload.voucher_signature.is_some() {
        return Err(UptoFailure::new(
            codes::UNEXPECTED_VOUCHER,
            &payload.from,
            "voucherSignature is claim-only",
        ));
    }
    let channel = resolve_payment_channel_config(requirements).map_err(|err| {
        UptoFailure::new(codes::PAYMENT_REQUIREMENTS, &payload.from, err.to_string())
    })?;
    let fee_payer = Pubkey::from_str(&channel.fee_payer).map_err(|err| {
        UptoFailure::new(codes::PAYMENT_REQUIREMENTS, &payload.from, err.to_string())
    })?;
    if fee_payer != provider.pubkey() {
        return Err(UptoFailure::new(
            "facilitator_mismatch",
            &payload.from,
            "feePayer is not this facilitator",
        ));
    }
    if payload.authorized_signer != channel.receiver_authorizer {
        return Err(UptoFailure::new(
            codes::RECEIVER_AUTHORIZER,
            &payload.from,
            "authorizedSigner != extra.receiverAuthorizer",
        ));
    }
    let max_amount = parse_u64(&payload.max_amount, codes::PAYLOAD_AMOUNT, &payload.from)?;
    let deposit = parse_u64(&payload.deposit, codes::DEPOSIT_NOT_CEILING, &payload.from)?;
    if deposit != max_amount {
        return Err(UptoFailure::new(
            codes::DEPOSIT_NOT_CEILING,
            &payload.from,
            "deposit must equal maxAmount",
        ));
    }
    let required_amount: u64 = requirements
        .amount
        .parse()
        .map_err(|_| UptoFailure::new(codes::PAYLOAD_AMOUNT, &payload.from, "invalid amount"))?;
    if required_amount != max_amount {
        return Err(UptoFailure::new(
            codes::AMOUNT_MISMATCH,
            &payload.from,
            "requirements.amount must equal payload.maxAmount",
        ));
    }
    check_lifetime(payload, requirements, config)?;
    let token_program = resolve_token_program(requirements).map_err(|err| {
        UptoFailure::new(codes::PAYMENT_REQUIREMENTS, &payload.from, err.to_string())
    })?;
    let transaction = decode_transaction(&payload.open_transaction)
        .map_err(|err| UptoFailure::new(codes::OPEN_TRANSACTION, &payload.from, err.to_string()))?;
    let open_slot = parse_u64(&payload.open_slot, codes::CHANNEL_SEED, &payload.from)?;
    let from = Pubkey::from_str(&payload.from)
        .map_err(|err| UptoFailure::new(codes::PAYER_MISMATCH, &payload.from, err.to_string()))?;
    let authorized = Pubkey::from_str(&payload.authorized_signer).map_err(|err| {
        UptoFailure::new(codes::RECEIVER_AUTHORIZER, &payload.from, err.to_string())
    })?;
    let mint = Pubkey::from_str(requirements.asset.as_str()).map_err(|err| {
        UptoFailure::new(codes::PAYMENT_REQUIREMENTS, &payload.from, err.to_string())
    })?;
    let result = verify_open_transaction(
        &transaction,
        &VerifyOpenExpected {
            authorized_signer: authorized,
            fee_payer,
            from,
            mint,
            token_program,
            payee: fee_payer,
            max_cap: max_amount,
            withdraw_delay: channel.withdraw_delay,
            open_slot,
            recipients: channel.splits.clone(),
            recent_slot: None,
            memo: parse_extra_memo(requirements.extra.as_ref()),
            max_compute_units: Some(config.max_compute_units),
            max_priority_fee_micro_lamports: Some(config.max_priority_fee_micro_lamports),
            max_required_signatures: config.max_required_signatures,
        },
    )
    .map_err(|err| UptoFailure::new(codes::OPEN_TRANSACTION, &payload.from, err.to_string()))?;
    if result.channel_id.to_string() != payload.channel_id {
        return Err(UptoFailure::new(
            codes::CHANNEL_ID,
            &payload.from,
            "channel PDA mismatch",
        ));
    }
    if result.salt.to_string() != payload.nonce {
        return Err(UptoFailure::new(
            codes::NONCE,
            &payload.from,
            "open salt != nonce",
        ));
    }
    if result.payer != from {
        return Err(UptoFailure::new(
            codes::PAYER_MISMATCH,
            &payload.from,
            "open payer != payload.from",
        ));
    }
    Ok(OpenAuth {
        payload: payload.clone(),
        channel,
        max_amount,
        token_program,
        transaction,
    })
}

fn check_lifetime(
    payload: &UptoSvmPayload,
    requirements: &r402_protocol::PaymentRequirements,
    config: &SolanaUptoFacilitatorConfig,
) -> Result<(), UptoFailure> {
    let now = unix_now().map_err(|err| UptoFailure::new(codes::EXPIRED, &payload.from, err))?;
    if now < payload.valid_after {
        return Err(UptoFailure::new(
            codes::NOT_YET_ACTIVE,
            &payload.from,
            "authorization is not yet active",
        ));
    }
    if payload.expires_at == 0 || now >= payload.expires_at {
        return Err(UptoFailure::new(
            codes::EXPIRED,
            &payload.from,
            "authorization expired",
        ));
    }
    let max_lifetime = i64::try_from(config.max_channel_lifetime_secs).unwrap_or(i64::MAX);
    if payload.expires_at.saturating_sub(now) > max_lifetime {
        return Err(UptoFailure::new(
            codes::CHANNEL_LIFETIME_EXCEEDED,
            &payload.from,
            "channel lifetime exceeds facilitator cap",
        ));
    }
    let timeout = i64::try_from(requirements.max_timeout_seconds).unwrap_or(i64::MAX);
    if payload.expires_at
        > now
            .saturating_add(timeout)
            .saturating_add(EXPIRES_AT_CLOCK_SKEW_SECS)
    {
        return Err(UptoFailure::new(
            codes::EXPIRES_AT_MISMATCH,
            &payload.from,
            "expiresAt exceeds now + maxTimeoutSeconds",
        ));
    }
    Ok(())
}

pub(crate) fn decode_transaction(b64: &str) -> Result<VersionedTransaction, FacilitatorError> {
    let raw = Base64Bytes(b64.as_bytes().to_vec())
        .decode()
        .map_err(|err| format_err(err.to_string()))?;
    bincode::deserialize(&raw).map_err(|err| format_err(err.to_string()))
}

pub(crate) fn parse_u64(raw: &str, code: &'static str, payer: &str) -> Result<u64, UptoFailure> {
    raw.parse()
        .map_err(|_| UptoFailure::new(code, payer, "not an unsigned integer"))
}

pub(crate) fn unix_now() -> Result<i64, String> {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|err| err.to_string())?
        .as_secs();
    i64::try_from(secs).map_err(|err| err.to_string())
}

fn format_err(err: impl std::fmt::Display) -> FacilitatorError {
    FacilitatorError::Verification(VerificationError::InvalidFormat(err.to_string()))
}
