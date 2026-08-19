//! Facilitator settlement for the Stellar exact scheme.

use std::future::Future;

use compact_str::CompactString;
use r402_core::cache::{Duplicate, SettlementCache};
use r402_core::error::ErrorReason;
use r402_core::wire::{Extensions, SettleResponse, VerifyResponse};
use serde_json::Value;
use stellar_rpc_client::GetTransactionResponse;

use super::verify::{VerifiedStellar, verify_for_settle};
use crate::DEFAULT_TIMEOUT_SECONDS;
use crate::chain::StellarFacilitatorError;
use crate::chain::rpc::StellarRpc;

/// Settles a verified Stellar payment.
///
/// `submit` rebuilds the envelope with the facilitator as source and submits it.
pub async fn settle_request<R, F, Fut>(
    rpc: &R,
    facilitator_addresses: &[String],
    cache: &SettlementCache,
    max_transaction_fee_stroops: u32,
    request: &Value,
    now_unix: u64,
    submit: F,
) -> SettleResponse
where
    R: StellarRpc,
    F: FnOnce(VerifiedStellar, u64, u64) -> Fut,
    Fut: Future<Output = Result<GetTransactionResponse, StellarFacilitatorError>> + Send,
{
    let network = request
        .get("paymentRequirements")
        .and_then(|r| r.get("network"))
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_owned();
    let max_timeout = request
        .get("paymentRequirements")
        .and_then(|r| r.get("maxTimeoutSeconds"))
        .and_then(Value::as_u64)
        .filter(|v| *v > 0)
        .unwrap_or(DEFAULT_TIMEOUT_SECONDS);

    let verified = match verify_for_settle(
        rpc,
        facilitator_addresses,
        max_transaction_fee_stroops,
        request,
    )
    .await
    {
        Ok(v) => v,
        Err(invalid) => {
            let response = invalid.into_response();
            return match response {
                VerifyResponse::Invalid { reason, payer, .. } => {
                    settle_failure(reason, &network, payer)
                }
                _ => settle_failure(
                    ErrorReason::from_wire("unexpected_verify_error"),
                    &network,
                    None,
                ),
            };
        }
    };
    let payer = verified.payer.clone();

    let tx_b64 = request
        .get("paymentPayload")
        .and_then(|p| p.get("payload"))
        .and_then(|p| p.get("transaction"))
        .and_then(Value::as_str);
    let Some(tx_b64) = tx_b64 else {
        return settle_failure(
            ErrorReason::from_wire("invalid_exact_stellar_payload_malformed"),
            &network,
            Some(payer),
        );
    };
    if cache.reserve(tx_b64) == Duplicate::Yes {
        return settle_failure(ErrorReason::DuplicateSettlement, &network, Some(payer));
    }

    match submit(verified, max_timeout, now_unix).await {
        Ok(confirm) => {
            if confirm.status == "SUCCESS" {
                let hash = confirm.tx_hash.unwrap_or_default();
                SettleResponse::Success {
                    payer,
                    transaction: hash.into(),
                    network: network.into(),
                    amount: request
                        .get("paymentRequirements")
                        .and_then(|r| r.get("amount"))
                        .and_then(Value::as_str)
                        .map(CompactString::from),
                    extensions: Extensions::new(),
                }
            } else {
                SettleResponse::Failure {
                    reason: ErrorReason::from_wire("settle_exact_stellar_transaction_failed"),
                    message: None,
                    payer: Some(payer),
                    network: network.into(),
                    extensions: Extensions::new(),
                }
            }
        }
        Err(StellarFacilitatorError::NoSigner) => settle_failure(
            ErrorReason::from_wire("settle_exact_stellar_signer_selection_failed"),
            &network,
            Some(payer),
        ),
        Err(StellarFacilitatorError::Rpc(err)) => {
            let reason = ErrorReason::from_wire(err.settle_reason());
            let mut message = err.to_string();
            if let Some(hash) = err.transaction_hash()
                && !message.contains(hash)
            {
                message = format!("{hash}: {message}");
            }
            SettleResponse::Failure {
                reason,
                message: Some(message.into()),
                payer: Some(payer),
                network: network.into(),
                extensions: Extensions::new(),
            }
        }
        Err(err) => SettleResponse::Failure {
            reason: ErrorReason::from_wire("unexpected_settle_error"),
            message: Some(err.to_string().into()),
            payer: Some(payer),
            network: network.into(),
            extensions: Extensions::new(),
        },
    }
}

fn settle_failure(
    reason: ErrorReason,
    network: &str,
    payer: Option<CompactString>,
) -> SettleResponse {
    SettleResponse::Failure {
        reason,
        message: None,
        payer,
        network: network.into(),
        extensions: Extensions::new(),
    }
}
