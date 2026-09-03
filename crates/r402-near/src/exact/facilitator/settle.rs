//! Facilitator settlement for the NEAR exact scheme.

use std::future::Future;

use compact_str::CompactString;
use r402_facilitator::{Duplicate, SettlementCache};
use r402_protocol::error::ErrorReason;
use r402_protocol::payment::{Extensions, SettleResponse, VerifyResponse};
use serde_json::Value;

use super::verify::{decode_signed_delegate, verify_request_json};
use crate::chain::rpc::{NearReceiptStatus, NearRpc, NearRpcError, NearSettlementOutcome};

/// Settles a verified NEAR payment.
///
/// `submit` wraps the signed delegate in an outer relayer transaction.
pub async fn settle_request<R, F, Fut>(
    rpc: &R,
    relayer_ids: &[String],
    cache: &SettlementCache,
    max_sponsored_gas: u64,
    expected_network: &str,
    request: &Value,
    submit: F,
) -> SettleResponse
where
    R: NearRpc,
    F: FnOnce(String, String) -> Fut,
    Fut: Future<Output = Result<NearSettlementOutcome, NearRpcError>> + Send,
{
    let network = request
        .get("paymentRequirements")
        .and_then(|r| r.get("network"))
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_owned();

    let verified = verify_request_json(
        rpc,
        relayer_ids,
        max_sponsored_gas,
        expected_network,
        request,
    )
    .await;
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

    let Some(relayer_id) = relayer_ids.first().cloned() else {
        return settle_failure(
            ErrorReason::from_wire("invalid_exact_near_no_relayer_configured"),
            &network,
            payer,
        );
    };

    let signed_b64 = request
        .get("paymentPayload")
        .and_then(|p| p.get("payload"))
        .and_then(|p| p.get("signedDelegateAction"))
        .and_then(Value::as_str);
    let Some(signed_b64) = signed_b64 else {
        return settle_failure(
            ErrorReason::from_wire("invalid_exact_near_payload_shape"),
            &network,
            payer,
        );
    };

    if decode_signed_delegate(signed_b64).is_err() {
        return settle_failure(
            ErrorReason::from_wire("invalid_exact_near_payload_signed_delegate_action"),
            &network,
            payer,
        );
    }

    if cache.reserve(signed_b64) == Duplicate::Yes {
        return settle_failure(ErrorReason::DuplicateSettlement, &network, payer);
    }

    match submit(relayer_id, signed_b64.to_owned()).await {
        Ok(outcome) => match outcome.inner_receipt {
            NearReceiptStatus::Success { .. } => SettleResponse::Success {
                payer: payer.unwrap_or_default(),
                transaction: outcome.transaction.into(),
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
            NearReceiptStatus::Failure { error } => SettleResponse::Failure {
                reason: ErrorReason::from_wire("settlement_failed"),
                message: Some(error.into()),
                transaction: CompactString::default(),
                payer,
                network: network.into(),
                extensions: Extensions::new(),
                extension_responses: Extensions::new(),
                extra: None,
            },
        },
        Err(err) => SettleResponse::Failure {
            reason: ErrorReason::from_wire("settlement_failed"),
            message: Some(err.to_string().into()),
            transaction: CompactString::default(),
            payer,
            network: network.into(),
            extensions: Extensions::new(),
            extension_responses: Extensions::new(),
            extra: None,
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
        transaction: CompactString::default(),
        payer,
        network: network.into(),
        extensions: Extensions::new(),
        extension_responses: Extensions::new(),
        extra: None,
    }
}
