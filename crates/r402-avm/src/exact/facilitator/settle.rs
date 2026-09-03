//! Facilitator settlement for the Algorand exact scheme.

use compact_str::CompactString;
use r402_facilitator::{Duplicate, SettlementCache};
use r402_protocol::error::ErrorReason;
use r402_protocol::payment::{Extensions, SettleResponse, VerifyResponse};
use serde_json::Value;

use super::verify::{decode_transaction_group, prepare_signed_group_with, verify_request_json};
use crate::chain::codec::{Transaction, decode_signed, txid_str};
use crate::chain::rpc::{AlgodError, AlgodRpc};
use crate::exact::error::AlgorandInvalid;

/// Settles a verified Algorand payment.
pub async fn settle_request<R, F, C, Fut>(
    rpc: &R,
    facilitator_addresses: &[String],
    cache: &SettlementCache,
    wait_rounds: u32,
    request: &Value,
    mut sign: F,
    confirm: C,
) -> SettleResponse
where
    R: AlgodRpc,
    F: FnMut(&Transaction, &str) -> Result<Vec<u8>, AlgorandInvalid>,
    C: FnOnce(String, u32) -> Fut,
    Fut: Future<Output = Result<(), AlgodError>> + Send,
{
    let network = request
        .get("paymentRequirements")
        .and_then(|r| r.get("network"))
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_owned();

    let verified = verify_request_json(rpc, facilitator_addresses, request, &mut sign).await;
    let payer = match verified {
        VerifyResponse::Valid { payer, .. } => payer,
        VerifyResponse::Invalid { payer, reason, .. } => {
            return settle_failure(
                reason.unwrap_or(ErrorReason::UnexpectedVerifyError),
                &network,
                payer,
            );
        }
        _ => {
            return settle_failure(
                ErrorReason::from_wire("unexpected_verify_error"),
                &network,
                None,
            );
        }
    };

    let payload = request.get("paymentPayload").and_then(|p| p.get("payload"));
    let Some(payload) = payload else {
        return settle_failure(
            ErrorReason::from_wire("invalid_exact_avm_payload"),
            &network,
            payer,
        );
    };
    let Some(payment_group) = payload.get("paymentGroup").and_then(Value::as_array) else {
        return settle_failure(
            ErrorReason::from_wire("invalid_exact_avm_payload"),
            &network,
            payer,
        );
    };
    let Some(payment_index) = payload.get("paymentIndex").and_then(Value::as_u64) else {
        return settle_failure(
            ErrorReason::from_wire("invalid_exact_avm_payload"),
            &network,
            payer,
        );
    };

    let cache_key = payment_group
        .iter()
        .filter_map(Value::as_str)
        .collect::<Vec<_>>()
        .join("|");
    if cache.reserve(cache_key) == Duplicate::Yes {
        return settle_failure(ErrorReason::DuplicateSettlement, &network, payer);
    }

    let decoded = match decode_transaction_group(payment_group, facilitator_addresses) {
        Ok(txns) => txns,
        Err(err) => {
            return settle_failure(err.reason, &network, payer);
        }
    };
    let prepared = match prepare_signed_group_with(
        &decoded,
        payment_group,
        facilitator_addresses,
        &mut sign,
    ) {
        Ok(txns) => txns,
        Err(err) => {
            return settle_failure(err.reason, &network, payer);
        }
    };

    let payment_idx = usize::try_from(payment_index).unwrap_or(usize::MAX);
    let payment_txid = prepared
        .get(payment_idx)
        .and_then(|bytes| decode_signed(bytes).ok())
        .map(|stxn| txid_str(&stxn.txn))
        .unwrap_or_default();

    if let Err(err) = rpc.send_group(&prepared).await {
        return SettleResponse::Failure {
            reason: ErrorReason::from_wire("invalid_exact_avm_settlement_failed"),
            message: Some(format!("Failed to submit transaction: {err}").into()),
            payer,
            transaction: CompactString::default(),
            network: network.into(),
            extensions: Extensions::new(),
            extension_responses: Extensions::new(),
            extra: None,
        };
    }

    if let Err(err) = confirm(payment_txid.clone(), wait_rounds).await {
        return SettleResponse::Failure {
            reason: ErrorReason::from_wire("invalid_exact_avm_confirmation_failed"),
            message: Some(format!("Transaction submitted but confirmation failed: {err}").into()),
            transaction: payment_txid.into(),
            payer,
            network: network.into(),
            extensions: Extensions::new(),
            extension_responses: Extensions::new(),
            extra: None,
        };
    }

    SettleResponse::Success {
        payer,
        transaction: payment_txid.into(),
        network: network.into(),
        amount: request
            .get("paymentRequirements")
            .and_then(|r| r.get("amount"))
            .and_then(Value::as_str)
            .map(CompactString::from),
        extensions: Extensions::new(),
        extension_responses: Extensions::new(),
        extra: None,
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
        transaction: CompactString::default(),
        payer,
        network: network.into(),
        extensions: Extensions::new(),
        extension_responses: Extensions::new(),
        extra: None,
    }
}
