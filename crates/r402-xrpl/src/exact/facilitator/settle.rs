//! Facilitator settlement for the XRPL exact scheme.

use compact_str::CompactString;
use r402_core::cache::{Duplicate, SettlementCache};
use r402_core::error::ErrorReason;
use r402_core::wire::{Extensions, SettleResponse, VerifyResponse};
use serde_json::Value;

use super::verify::verify_request_json;
use crate::SETTLEMENT_TTL_MS;
use crate::chain::codec::{decode_signed_tx_blob, signed_tx_hash};
use crate::chain::rpc::{XrplRpc, XrplRpcError, XrplTxResult};

/// Settles a verified XRPL payment by submitting the signed blob.
pub async fn settle_request<R, F, Fut>(
    rpc: &R,
    cache: &SettlementCache,
    max_fee_drops: u64,
    request: &Value,
    submit: F,
) -> SettleResponse
where
    R: XrplRpc,
    F: FnOnce(String, u32) -> Fut,
    Fut: Future<Output = Result<XrplTxResult, XrplRpcError>> + Send,
{
    let network = request
        .get("paymentRequirements")
        .and_then(|r| r.get("network"))
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_owned();

    let verified = verify_request_json(rpc, max_fee_drops, request).await;
    let payer = match verified {
        VerifyResponse::Valid { payer, .. } => Some(payer),
        VerifyResponse::Invalid { payer, reason, .. } => {
            return settle_failure(reason, &network, payer);
        }
        _ => {
            return settle_failure(
                ErrorReason::from_wire("unexpected_verify_error"),
                &network,
                None,
            );
        }
    };

    let signed_blob = request
        .get("paymentPayload")
        .and_then(|p| p.get("payload"))
        .and_then(|p| p.get("signedTxBlob"))
        .and_then(Value::as_str);
    let Some(signed_blob) = signed_blob else {
        return settle_failure(
            ErrorReason::from_wire("invalid_exact_xrpl_payload_shape"),
            &network,
            payer,
        );
    };

    let Ok(hash) = signed_tx_hash(signed_blob) else {
        return settle_failure(
            ErrorReason::from_wire("invalid_exact_xrpl_payload_shape"),
            &network,
            payer,
        );
    };

    let last_ledger = decode_signed_tx_blob(signed_blob)
        .ok()
        .and_then(|tx| tx.get("LastLedgerSequence").and_then(Value::as_u64))
        .and_then(|v| u32::try_from(v).ok())
        .unwrap_or(0);

    let max_timeout = request
        .get("paymentRequirements")
        .and_then(|r| r.get("maxTimeoutSeconds"))
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let _ttl_ms = max_timeout
        .saturating_mul(1000)
        .saturating_add(SETTLEMENT_TTL_MS);

    if cache.reserve(&hash) == Duplicate::Yes {
        return settle_failure(ErrorReason::DuplicateSettlement, &network, payer);
    }

    match submit(signed_blob.to_owned(), last_ledger).await {
        Ok(outcome) if outcome.validated && outcome.result_code == "tesSUCCESS" => {
            SettleResponse::Success {
                payer: payer.unwrap_or_default(),
                transaction: outcome.hash.into(),
                network: network.into(),
                amount: request
                    .get("paymentRequirements")
                    .and_then(|r| r.get("amount"))
                    .and_then(Value::as_str)
                    .map(CompactString::from),
                extensions: Extensions::new(),
            }
        }
        Ok(outcome) => SettleResponse::Failure {
            reason: ErrorReason::from_wire(&format!("transaction_failed: {}", outcome.result_code)),
            message: None,
            payer,
            network: network.into(),
            extensions: Extensions::new(),
        },
        Err(err) => SettleResponse::Failure {
            reason: ErrorReason::from_wire(&format!("transaction_failed: {err}")),
            message: Some(err.to_string().into()),
            payer,
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
