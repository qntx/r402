//! Facilitator settlement for the Aptos exact scheme.

use compact_str::CompactString;
use r402_facilitator::{Duplicate, SettlementCache};
use r402_protocol::error::ErrorReason;
use r402_protocol::payment::{Extensions, SettleResponse, VerifyResponse};
use serde_json::Value;

use super::AptosFacilitatorOps;
use super::verify::verify_request_json;

/// Settles a verified Aptos payment.
pub async fn settle_request<P: AptosFacilitatorOps>(
    provider: &P,
    cache: &SettlementCache,
    request: &Value,
) -> SettleResponse {
    let network = request
        .get("paymentRequirements")
        .and_then(|r| r.get("network"))
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_owned();

    let verified = verify_request_json(provider, request).await;
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

    let fee_payer = request
        .get("paymentRequirements")
        .and_then(|r| r.get("extra"))
        .and_then(|e| e.get("feePayer"))
        .and_then(Value::as_str);

    let Some(transaction_b64) = request
        .get("paymentPayload")
        .and_then(|p| p.get("payload"))
        .and_then(|p| p.get("transaction"))
        .and_then(Value::as_str)
    else {
        return settle_failure(
            ErrorReason::from_wire("invalid_exact_aptos_payload_verification_error"),
            &network,
            payer,
        );
    };

    if cache.reserve(transaction_b64) == Duplicate::Yes {
        return settle_failure(ErrorReason::DuplicateSettlement, &network, payer);
    }

    match provider
        .sign_and_submit(transaction_b64, fee_payer, &network)
        .await
    {
        Ok(transaction) => SettleResponse::Success {
            payer,
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
        Err(err) => SettleResponse::Failure {
            reason: ErrorReason::from_wire("transaction_failed"),
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
