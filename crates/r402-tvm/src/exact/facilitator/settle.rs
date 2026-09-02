//! Facilitator settlement for the TON exact scheme.

use compact_str::CompactString;
use r402_core::cache::{Duplicate, SettlementCache};
use r402_core::error::ErrorReason;
use r402_core::wire::{Extensions, SettleResponse};
use serde_json::Value;

use super::verify::verify_request_json;
use crate::chain::TvmRelayRequest;
use crate::chain::rpc::TvmRpc;
use crate::exact::error::{ERR_EXACT_TVM_DUPLICATE_SETTLEMENT, ERR_EXACT_TVM_TRANSACTION_FAILED};
use crate::exact::settlement_batcher::{QueuedSettlement, SettlementBatcher};

/// Settles a verified TON payment through the Highload V3 batcher.
pub async fn settle_request<R>(
    rpc: &R,
    facilitator_addresses: &[String],
    cache: &SettlementCache,
    batcher: &SettlementBatcher,
    request: &Value,
    now_unix: u64,
    emulate: impl AsyncFn(&TvmRelayRequest) -> Result<Value, String>,
) -> SettleResponse
where
    R: TvmRpc,
{
    let network = request
        .get("paymentRequirements")
        .and_then(|r| r.get("network"))
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_owned();

    let verified =
        match verify_request_json(rpc, facilitator_addresses, request, now_unix, emulate).await {
            Ok(v) => v,
            Err(invalid) => {
                return settle_failure(invalid.reason, invalid.message, &network, invalid.payer);
            }
        };

    if cache.reserve(verified.settlement.settlement_hash.clone()) == Duplicate::Yes {
        return settle_failure(
            ErrorReason::from_wire(ERR_EXACT_TVM_DUPLICATE_SETTLEMENT),
            None,
            &network,
            Some(verified.payer),
        );
    }

    let payer = verified.payer.clone();
    let result = batcher
        .enqueue(
            network.clone(),
            QueuedSettlement {
                settlement_hash: verified.settlement.settlement_hash.clone(),
                settlement: verified.settlement,
                relay_request: verified.relay_request,
            },
        )
        .await;

    if result.success {
        SettleResponse::Success {
            payer,
            transaction: result.transaction.unwrap_or_default().into(),
            network: network.into(),
            amount: request
                .get("paymentRequirements")
                .and_then(|r| r.get("amount"))
                .and_then(Value::as_str)
                .map(CompactString::from),
            extensions: Extensions::new(),
            extra: None,
        }
    } else {
        settle_failure(
            ErrorReason::from_wire(
                result
                    .error_reason
                    .as_deref()
                    .unwrap_or(ERR_EXACT_TVM_TRANSACTION_FAILED),
            ),
            result.error_message.map(CompactString::from),
            &network,
            Some(payer),
        )
    }
}

fn settle_failure(
    reason: ErrorReason,
    message: Option<CompactString>,
    network: &str,
    payer: Option<CompactString>,
) -> SettleResponse {
    SettleResponse::Failure {
        transaction: Default::default(),
        reason,
        message,
        payer,
        network: network.into(),
        extensions: Extensions::new(),
        extra: None,
    }
}
