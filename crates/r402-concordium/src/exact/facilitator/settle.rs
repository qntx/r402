//! Facilitator settlement for the Concordium exact scheme.

use std::time::Duration;

use compact_str::CompactString;
use r402_facilitator::SettlementCache;
use r402_protocol::error::ErrorReason;
use r402_protocol::payment::{Extensions, SettleResponse, VerifyResponse};
use serde_json::Value;

use super::verify::{parse_transaction, verify_request_json};
use crate::chain::account::ConcordiumAddress;
use crate::chain::rpc::ConcordiumNode;
use crate::chain::signer::ConcordiumSigner;
use crate::chain::tx::add_sponsor_signature;
use crate::exact::payload::TransactionStatus;

/// Settles a verified Concordium payment.
#[allow(
    clippy::too_many_arguments,
    clippy::too_many_lines,
    reason = "settle routing matches official facilitator scheme.ts"
)]
pub async fn settle_request<N: ConcordiumNode>(
    node: &N,
    signers: &[ConcordiumSigner],
    facilitator_addresses: &[String],
    cache: &SettlementCache,
    request: &Value,
    max_expiry_offset_seconds: u64,
    require_finalization: bool,
    finalization_timeout: Duration,
) -> SettleResponse {
    let network = request
        .get("paymentRequirements")
        .and_then(|r| r.get("network"))
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_owned();

    let verified = verify_request_json(
        node,
        facilitator_addresses,
        request,
        max_expiry_offset_seconds,
    )
    .await;
    let payer = match verified {
        VerifyResponse::Valid { payer, .. } => payer,
        VerifyResponse::Invalid { payer, reason, .. } => {
            return settle_failure(
                reason.unwrap_or_else(|| ErrorReason::from_wire("verification_failed")),
                &network,
                payer,
                CompactString::default(),
            );
        }
        _ => {
            return settle_failure(
                ErrorReason::from_wire("unexpected_verify_error"),
                &network,
                None,
                CompactString::default(),
            );
        }
    };
    let Some(verified_payer) = payer.filter(|p| !p.is_empty()) else {
        return settle_failure(
            ErrorReason::from_wire("missing_payer"),
            &network,
            None,
            CompactString::default(),
        );
    };

    let inner = request
        .get("paymentPayload")
        .and_then(|p| p.get("payload"))
        .unwrap_or(&Value::Null);
    let Ok(tx) = parse_transaction(inner) else {
        return settle_failure(
            ErrorReason::from_wire("invalid_transaction_format"),
            &network,
            Some(verified_payer),
            CompactString::default(),
        );
    };

    let fee_payer = request
        .get("paymentRequirements")
        .and_then(|r| r.get("extra"))
        .and_then(|e| e.get("feePayer"))
        .and_then(Value::as_str)
        .unwrap_or("");
    if fee_payer.is_empty() {
        return settle_failure(
            ErrorReason::from_wire("missing_fee_payer"),
            &network,
            Some(verified_payer),
            CompactString::default(),
        );
    }
    let Ok(fee_addr) = ConcordiumAddress::decode(fee_payer) else {
        return settle_failure(
            ErrorReason::from_wire("fee_payer_not_managed_by_facilitator"),
            &network,
            Some(verified_payer),
            CompactString::default(),
        );
    };
    let Some(sponsor) = signers.iter().find(|s| s.address() == fee_addr) else {
        return settle_failure(
            ErrorReason::from_wire("fee_payer_not_managed_by_facilitator"),
            &network,
            Some(verified_payer),
            CompactString::default(),
        );
    };

    let cache_key = format!("{}:{}", tx.header.sender, tx.header.nonce);
    if cache.reserve(&cache_key) == r402_facilitator::Duplicate::Yes {
        return settle_failure(
            ErrorReason::DuplicateSettlement,
            &network,
            Some(verified_payer),
            CompactString::default(),
        );
    }

    let signed = match add_sponsor_signature(&tx, sponsor) {
        Ok(tx) => tx,
        Err(e) => {
            cache.release(&cache_key);
            return settle_failure(
                ErrorReason::from_wire(&format!("sponsor_signing_failed: {e}")),
                &network,
                Some(verified_payer),
                CompactString::default(),
            );
        }
    };

    let tx_hash = match node.send_v1(signed).await {
        Ok(hash) => hash,
        Err(e) => {
            cache.release(&cache_key);
            return settle_failure(
                ErrorReason::from_wire(&format!("submission_failed: {e}")),
                &network,
                Some(verified_payer),
                CompactString::default(),
            );
        }
    };

    let info = match node.wait_finalized(&tx_hash, finalization_timeout).await {
        Ok(info) => info,
        Err(e) => {
            return settle_failure(
                ErrorReason::from_wire(&format!("finalization_failed: {e}")),
                &network,
                Some(verified_payer),
                CompactString::from(tx_hash.as_str()),
            );
        }
    };

    if require_finalization && info.status != TransactionStatus::Finalized {
        return settle_failure(
            ErrorReason::from_wire("finalization_timeout"),
            &network,
            Some(verified_payer),
            CompactString::from(tx_hash.as_str()),
        );
    }
    if !info.sender.is_empty() && info.sender != verified_payer.as_str() {
        return settle_failure(
            ErrorReason::from_wire("on_chain_sender_mismatch"),
            &network,
            Some(verified_payer),
            CompactString::from(tx_hash.as_str()),
        );
    }
    let pay_to = request
        .get("paymentRequirements")
        .and_then(|r| r.get("payTo"))
        .and_then(Value::as_str)
        .unwrap_or("");
    if info.recipient.as_deref() != Some(pay_to) {
        return settle_failure(
            ErrorReason::from_wire("on_chain_recipient_mismatch"),
            &network,
            Some(verified_payer),
            CompactString::from(tx_hash.as_str()),
        );
    }

    SettleResponse::Success {
        payer: Some(verified_payer),
        transaction: CompactString::from(tx_hash.as_str()),
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
    transaction: CompactString,
) -> SettleResponse {
    SettleResponse::Failure {
        reason,
        message: None,
        transaction,
        payer,
        network: network.into(),
        extensions: Extensions::new(),
        extension_responses: Extensions::new(),
        extra: None,
    }
}
