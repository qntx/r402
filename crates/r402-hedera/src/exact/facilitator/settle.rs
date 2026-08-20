//! Facilitator settlement for the Hedera exact scheme.

use compact_str::CompactString;
use r402_core::cache::{Duplicate, SettlementCache};
use r402_core::error::ErrorReason;
use r402_core::wire::{Extensions, SettleResponse, VerifyResponse};
use serde_json::Value;

use super::verify::verify_request_json;
use super::{AliasPolicy, HederaFacilitatorOps};

/// Settles a verified Hedera payment.
pub async fn settle_request<P: HederaFacilitatorOps>(
    provider: &P,
    alias_policy: AliasPolicy,
    cache: &SettlementCache,
    request: &Value,
) -> SettleResponse {
    let network = request
        .get("paymentRequirements")
        .and_then(|r| r.get("network"))
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_owned();

    let verified = verify_request_json(provider, alias_policy, request).await;
    let payer = match verified {
        VerifyResponse::Valid { payer, .. } => Some(payer),
        VerifyResponse::Invalid {
            payer,
            reason,
            message,
            ..
        } => {
            return settle_failure(reason, message, &network, payer);
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

    let fee_payer = request
        .get("paymentRequirements")
        .and_then(|r| r.get("extra"))
        .and_then(|e| e.get("feePayer"))
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_owned();

    let Some(transaction_b64) = request
        .get("paymentPayload")
        .and_then(|p| p.get("payload"))
        .and_then(|p| p.get("transaction"))
        .and_then(Value::as_str)
    else {
        return settle_failure(
            ErrorReason::from_wire("invalid_exact_hedera_payload_transaction_could_not_be_decoded"),
            None,
            &network,
            payer,
        );
    };

    if cache.reserve(transaction_b64) == Duplicate::Yes {
        return settle_failure(ErrorReason::DuplicateSettlement, None, &network, payer);
    }

    match provider
        .sign_and_submit(transaction_b64, &fee_payer, &network)
        .await
    {
        Ok(transaction_id) => SettleResponse::Success {
            payer: payer.unwrap_or_else(|| CompactString::from(fee_payer)),
            transaction: transaction_id.into(),
            network: network.into(),
            amount: request
                .get("paymentRequirements")
                .and_then(|r| r.get("amount"))
                .and_then(Value::as_str)
                .map(CompactString::from),
            extensions: Extensions::new(),
        },
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
