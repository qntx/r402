//! On-chain settlement for the EIP-155 upto scheme.
//!
//! `actualAmount == 0` skips the on-chain transaction (authorisation expires unused).

use alloy_primitives::{Bytes, TxHash, U256};
#[cfg(feature = "telemetry")]
use tracing_core::Level;

use super::contract::IX402UptoPermit2Proxy;
use super::verify::PreparedUptoPermit2;
use super::verify_permit2::{assert_eip2612_supported_signature_kind, build_eip2612_abi};
use crate::chain::{Eip155MetaTransactionProvider, MetaTransaction};
use crate::erc20_approval::relay_erc20_approval;
use crate::error::Eip155ExactError;
use crate::signature::{ClassifiedSignature, classify_with_code};
use crate::upto::X402_UPTO_PERMIT2_PROXY;

/// Outcome of a settle call.
#[derive(Debug)]
pub(super) enum UptoSettleOutcome {
    /// Zero-settle: `actual_amount == 0`, authorisation expires unused.
    ZeroSettle,
    /// Transaction landed on-chain with the returned hash.
    OnChain(TxHash),
}

/// Settle a verified upto permit2 payment for `actual_amount` tokens.
///
/// Callers MUST have already validated `actual_amount <= prepared.max_amount`.
#[allow(
    clippy::cognitive_complexity,
    reason = "linear settle pipeline: zero-shortcut -> permit build -> signature dispatch -> tx submit"
)]
pub(super) async fn settle_permit2_upto<P, E>(
    provider: &P,
    prepared: &PreparedUptoPermit2,
    actual_amount: U256,
    data_suffix: &[u8],
) -> Result<UptoSettleOutcome, Eip155ExactError>
where
    P: Eip155MetaTransactionProvider<Error = E> + Sync,
    Eip155ExactError: From<E>,
{
    if actual_amount.is_zero() {
        #[cfg(feature = "telemetry")]
        tracing::event!(Level::INFO, "upto zero-settle: skipping on-chain tx");
        return Ok(UptoSettleOutcome::ZeroSettle);
    }

    if prepared.eip2612.is_none()
        && let Some(approval) = prepared.erc20_approval.as_ref()
    {
        let _ = relay_erc20_approval(
            provider,
            approval,
            prepared.from,
            actual_amount,
            prepared.facilitator,
        )
        .await?;
    }

    let classified = classify_with_code(
        provider.inner(),
        prepared.from,
        prepared.structured_signature.clone(),
        &prepared.eip712_hash,
    )
    .await?;
    assert_eip2612_supported_signature_kind(&classified, prepared.eip2612.is_some())?;

    let inner = provider.inner();
    let proxy = IX402UptoPermit2Proxy::new(X402_UPTO_PERMIT2_PROXY, inner);
    let permit = IX402UptoPermit2Proxy::Permit {
        permitted: IX402UptoPermit2Proxy::TokenPermissions {
            token: prepared.token,
            amount: prepared.max_amount,
        },
        nonce: prepared.nonce,
        deadline: prepared.deadline,
    };
    let witness = IX402UptoPermit2Proxy::Witness {
        to: prepared.to,
        facilitator: prepared.facilitator,
        validAfter: prepared.valid_after,
    };

    let sig_bytes: Bytes = match &classified {
        ClassifiedSignature::EIP6492 { original, .. } => original.clone(),
        ClassifiedSignature::Eoa(sig) => sig.as_bytes().into(),
        ClassifiedSignature::EIP1271(bytes) => bytes.clone(),
    };

    let calldata = match prepared.eip2612.as_ref() {
        Some(eip2612) => proxy
            .settleWithPermit(
                build_eip2612_abi(eip2612)?,
                permit,
                actual_amount,
                prepared.from,
                witness,
                sig_bytes,
            )
            .calldata()
            .clone(),
        None => proxy
            .settle(permit, actual_amount, prepared.from, witness, sig_bytes)
            .calldata()
            .clone(),
    };

    let tx_fut = Eip155MetaTransactionProvider::send_transaction(
        provider,
        // Pin sender to `witness.facilitator` so `msg.sender == witness.facilitator`.
        MetaTransaction {
            to: X402_UPTO_PERMIT2_PROXY,
            calldata,
            confirmations: 1,
            from: Some(prepared.facilitator),
            value: U256::ZERO,
        }
        .with_data_suffix(data_suffix),
    );
    let receipt = traced!(
        tx_fut,
        tracing::info_span!(
            "settle_upto_permit2",
            from = %prepared.from,
            to = %prepared.to,
            token = %prepared.token,
            max_amount = %prepared.max_amount,
            actual_amount = %actual_amount,
            otel.kind = "client",
        )
    )?;

    if receipt.status() {
        #[cfg(feature = "telemetry")]
        tracing::event!(Level::INFO, status = "ok", tx = %receipt.transaction_hash, "upto Permit2 settle succeeded");
        Ok(UptoSettleOutcome::OnChain(receipt.transaction_hash))
    } else {
        #[cfg(feature = "telemetry")]
        tracing::event!(Level::WARN, status = "failed", tx = %receipt.transaction_hash, "upto Permit2 settle failed");
        Err(Eip155ExactError::TransactionReverted(
            receipt.transaction_hash,
        ))
    }
}
