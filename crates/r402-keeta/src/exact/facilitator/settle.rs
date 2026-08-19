//! Facilitator settlement for the Keeta exact scheme.

use compact_str::CompactString;
use r402_core::error::ErrorReason;
use r402_core::wire::{Extensions, SettleResponse, VerifyResponse};
use serde_json::Value;

use super::queue::{QueueError, SettlementQueue};
use super::verify::{decode_block, verify_request_json};
use crate::chain::provider::KeetaPreflight;

/// Settles a verified Keeta payment through the per-fee-payer queue.
pub async fn settle_request<P: KeetaPreflight>(
    preflight: &P,
    fee_payer_ids: &[String],
    queue: &SettlementQueue,
    request: &Value,
) -> SettleResponse {
    let network = request
        .get("paymentRequirements")
        .and_then(|r| r.get("network"))
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_owned();

    let verified = verify_request_json(preflight, fee_payer_ids, request).await;
    let payer = match verified {
        VerifyResponse::Valid { payer, .. } => Some(payer),
        VerifyResponse::Invalid { payer, reason, .. } => {
            return settle_failure(reason, None, &network, payer);
        }
        _ => {
            return settle_failure(
                ErrorReason::from_wire("unexpected_verify_error"),
                None,
                &network,
                None,
            );
        }
    };

    let Some(fee_payer) = queue.pick_fee_payer() else {
        return settle_failure(
            ErrorReason::from_wire("transaction_failed"),
            Some("no fee payer addresses available".into()),
            &network,
            payer,
        );
    };

    let Some(block_b64) = request
        .get("paymentPayload")
        .and_then(|p| p.get("payload"))
        .and_then(|p| p.get("block"))
        .and_then(Value::as_str)
    else {
        return settle_failure(
            ErrorReason::from_wire("invalid_exact_keeta_payload_block_could_not_be_decoded"),
            None,
            &network,
            payer,
        );
    };

    let Ok(block) = decode_block(block_b64) else {
        return settle_failure(
            ErrorReason::from_wire("invalid_exact_keeta_payload_block_could_not_be_decoded"),
            None,
            &network,
            payer,
        );
    };

    match queue.enqueue(&fee_payer, block).await {
        Ok(transaction) => SettleResponse::Success {
            payer: payer.unwrap_or_default(),
            transaction: transaction.into(),
            network: network.into(),
            amount: request
                .get("paymentRequirements")
                .and_then(|r| r.get("amount"))
                .and_then(Value::as_str)
                .map(CompactString::from),
            extensions: Extensions::new(),
        },
        Err(QueueError::Duplicate) => settle_failure(
            ErrorReason::from_wire("duplicate_block"),
            None,
            &network,
            payer,
        ),
        Err(err) => settle_failure(
            ErrorReason::from_wire("transaction_failed"),
            Some(err.to_string().into()),
            &network,
            payer,
        ),
    }
}

fn settle_failure(
    reason: ErrorReason,
    message: Option<CompactString>,
    network: &str,
    payer: Option<CompactString>,
) -> SettleResponse {
    SettleResponse::Failure {
        reason,
        message,
        payer,
        network: network.into(),
        extensions: Extensions::new(),
    }
}
