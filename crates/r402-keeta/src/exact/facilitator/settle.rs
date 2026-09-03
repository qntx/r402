//! Facilitator settlement for the Keeta exact scheme.

use compact_str::CompactString;
use keetanetwork_block::Hashable;
use r402_facilitator::{Duplicate, SettlementCache};
use r402_protocol::error::ErrorReason;
use r402_protocol::payment::{Extensions, SettleResponse, VerifyResponse};
use serde_json::Value;

use super::queue::{QueueError, SettlementQueue};
use super::verify::{decode_block, verify_request_json};
use crate::chain::provider::KeetaPreflight;

/// Settles a verified Keeta payment through the per-account queue.
pub async fn settle_request<P: KeetaPreflight>(
    preflight: &P,
    fee_payer_ids: &[String],
    cache: &SettlementCache,
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

    if cache.reserve(block.hash().to_string()) == Duplicate::Yes {
        return settle_failure(ErrorReason::DuplicateSettlement, None, &network, payer);
    }

    match queue.enqueue(block).await {
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
            extension_responses: Extensions::new(),
            extra: None,
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
        transaction: CompactString::default(),
        reason,
        message,
        payer,
        network: network.into(),
        extensions: Extensions::new(),
        extension_responses: Extensions::new(),
        extra: None,
    }
}
