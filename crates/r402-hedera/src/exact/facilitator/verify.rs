//! Facilitator verification for the Hedera exact scheme.

use compact_str::CompactString;
use r402_protocol::payment::VerifyResponse;
use serde_json::Value;

use super::{AliasPolicy, HederaFacilitatorOps};
use crate::chain::tx::{
    InspectedHederaTransaction, asset_transfers, get_positive_receivers, has_negative_transfer,
    infer_payers, inspect_hedera_transaction, sum_transfers,
};
use crate::chain::{
    hedera_account_ids_equal, is_entity_id, is_hbar_asset, is_hedera_network, is_valid_asset,
};
use crate::exact::error::{HederaInvalid, invalid};

/// Verifies a raw facilitator verify/settle request JSON.
pub async fn verify_request_json<P: HederaFacilitatorOps>(
    provider: &P,
    alias_policy: AliasPolicy,
    request: &Value,
) -> VerifyResponse {
    match verify_inner(provider, alias_policy, request).await {
        Ok(payer) => VerifyResponse::valid(payer),
        Err(invalid) => invalid.into_response(),
    }
}

#[allow(
    clippy::too_many_lines,
    clippy::cognitive_complexity,
    reason = "sequential verify checklist must stay in one function"
)]
async fn verify_inner<P: HederaFacilitatorOps>(
    provider: &P,
    alias_policy: AliasPolicy,
    request: &Value,
) -> Result<CompactString, HederaInvalid> {
    let payload = request
        .get("paymentPayload")
        .ok_or_else(|| invalid("invalid_payload"))?;
    let requirements = request
        .get("paymentRequirements")
        .ok_or_else(|| invalid("invalid_payment_requirements"))?;

    let payload_version = payload.get("x402Version").and_then(Value::as_u64);
    if payload_version != Some(2) {
        return Err(invalid("invalid_x402_version"));
    }
    let accepted = payload
        .get("accepted")
        .ok_or_else(|| invalid("invalid_payload"))?;
    let accepted_scheme = accepted.get("scheme").and_then(Value::as_str);
    let req_scheme = requirements.get("scheme").and_then(Value::as_str);
    if accepted_scheme != Some("exact") || req_scheme != Some("exact") {
        return Err(invalid("unsupported_scheme"));
    }

    let req_network = requirements
        .get("network")
        .and_then(Value::as_str)
        .unwrap_or("");
    let accepted_network = accepted
        .get("network")
        .and_then(Value::as_str)
        .unwrap_or("");
    if accepted_network != req_network {
        return Err(invalid("network_mismatch"));
    }
    if !is_hedera_network(req_network) {
        return Err(invalid("network_mismatch"));
    }

    if accepted.get("asset") != requirements.get("asset")
        || accepted.get("amount") != requirements.get("amount")
        || accepted.get("payTo") != requirements.get("payTo")
        || accepted.get("maxTimeoutSeconds") != requirements.get("maxTimeoutSeconds")
        || accepted.get("extra").and_then(|e| e.get("feePayer"))
            != requirements.get("extra").and_then(|e| e.get("feePayer"))
    {
        return Err(invalid("accepted_payment_requirements_mismatch"));
    }

    let asset = requirements
        .get("asset")
        .and_then(Value::as_str)
        .unwrap_or("");
    if !is_valid_asset(asset) {
        return Err(invalid("invalid_asset"));
    }

    let pay_to = requirements
        .get("payTo")
        .and_then(Value::as_str)
        .unwrap_or("");
    if !is_valid_pay_to_format(pay_to, alias_policy) {
        return Err(invalid("invalid_exact_hedera_payload_pay_to"));
    }

    let amount = requirements
        .get("amount")
        .and_then(Value::as_str)
        .unwrap_or("");
    if amount.is_empty() || !amount.bytes().all(|b| b.is_ascii_digit()) {
        return Err(invalid("invalid_amount"));
    }

    let fee_payer = requirements
        .get("extra")
        .and_then(|e| e.get("feePayer"))
        .and_then(Value::as_str)
        .unwrap_or("");
    if !is_entity_id(fee_payer) {
        return Err(invalid("invalid_exact_hedera_payload_missing_fee_payer"));
    }
    let managed = provider.fee_payer_ids();
    if !managed
        .iter()
        .any(|id| hedera_account_ids_equal(id, fee_payer))
    {
        return Err(invalid("fee_payer_not_managed_by_facilitator"));
    }

    let transaction_b64 = payload
        .get("payload")
        .and_then(|p| p.get("transaction"))
        .and_then(Value::as_str)
        .unwrap_or("");
    if transaction_b64.is_empty() {
        return Err(invalid(
            "invalid_exact_hedera_payload_transaction_could_not_be_decoded",
        ));
    }

    let inspected = inspect_hedera_transaction(transaction_b64)
        .map_err(|_| invalid("invalid_exact_hedera_payload_transaction_could_not_be_decoded"))?;
    validate_inspected_shape(&inspected)?;
    validate_transfer_semantics(&inspected, requirements, fee_payer, asset, pay_to, amount)?;

    let transfers = asset_transfers(&inspected, asset)
        .ok_or_else(|| invalid("invalid_exact_hedera_payload_asset_mismatch"))?;
    let payers = infer_payers(transfers);
    let payer = CompactString::from(payers.first().map_or("", |(id, _)| id.as_str()));

    validate_pay_to_policy(provider, alias_policy, pay_to, req_network).await?;

    for (sender, _) in &payers {
        let signature = provider
            .verify_payer_signature(sender, transaction_b64, req_network)
            .await
            .map_err(|e| {
                invalid("invalid_exact_hedera_payload_signature_invalid")
                    .with_payer(payer.as_str())
                    .with_message(e.to_string())
            })?;
        if !signature.ok {
            let mut err = invalid("invalid_exact_hedera_payload_signature_invalid")
                .with_payer(payer.as_str());
            if let Some(message) = signature.invalid_message() {
                err = err.with_message(message);
            }
            return Err(err);
        }
    }

    for (sender, sender_amount) in &payers {
        let preflight = provider
            .preflight_transfer(sender, pay_to, asset, sender_amount, req_network)
            .await
            .map_err(|e| {
                invalid("invalid_exact_hedera_payload_preflight_failed")
                    .with_payer(payer.as_str())
                    .with_message(e.to_string())
            })?;
        if !preflight.ok {
            let mut err =
                invalid("invalid_exact_hedera_payload_preflight_failed").with_payer(payer.as_str());
            if let Some(message) = preflight.invalid_message() {
                err = err.with_message(message);
            }
            return Err(err);
        }
    }

    Ok(payer)
}

fn validate_inspected_shape(inspected: &InspectedHederaTransaction) -> Result<(), HederaInvalid> {
    if inspected.transaction_id.is_empty() || inspected.transaction_id_account_id.is_empty() {
        return Err(invalid(
            "invalid_exact_hedera_payload_transaction_invalid_shape",
        ));
    }
    if !is_entity_id(&inspected.transaction_id_account_id) {
        return Err(invalid(
            "invalid_exact_hedera_payload_transaction_invalid_shape",
        ));
    }
    Ok(())
}

fn validate_transfer_semantics(
    inspected: &InspectedHederaTransaction,
    _requirements: &Value,
    fee_payer: &str,
    asset: &str,
    pay_to: &str,
    amount: &str,
) -> Result<(), HederaInvalid> {
    if inspected.transaction_id_account_id != fee_payer {
        return Err(invalid("invalid_exact_hedera_payload_fee_payer_mismatch"));
    }
    if inspected.has_non_transfer_operations {
        return Err(invalid(
            "invalid_exact_hedera_payload_contains_non_transfer_ops",
        ));
    }
    if sum_transfers(&inspected.hbar_transfers) != 0 {
        return Err(invalid("invalid_exact_hedera_payload_hbar_sum_non_zero"));
    }
    if has_negative_transfer(&inspected.hbar_transfers, fee_payer) {
        return Err(invalid(
            "invalid_exact_hedera_payload_fee_payer_transferring_hbar",
        ));
    }
    if !is_hbar_asset(asset) && !inspected.hbar_transfers.is_empty() {
        return Err(invalid(
            "invalid_exact_hedera_payload_unexpected_hbar_transfers",
        ));
    }

    let Some(transfers) = asset_transfers(inspected, asset) else {
        return Err(invalid("invalid_exact_hedera_payload_asset_mismatch"));
    };
    if sum_transfers(transfers) != 0 {
        return Err(invalid("invalid_exact_hedera_payload_asset_sum_non_zero"));
    }
    if has_negative_transfer(transfers, fee_payer) {
        return Err(invalid(
            "invalid_exact_hedera_payload_fee_payer_transferring_funds",
        ));
    }

    let required: i128 = amount.parse().unwrap_or(-1);
    let net_to_pay_to: i128 = transfers
        .iter()
        .filter(|e| hedera_account_ids_equal(&e.account_id, pay_to))
        .map(|e| e.amount.parse::<i128>().unwrap_or(0))
        .sum();
    if net_to_pay_to != required {
        return Err(invalid("invalid_exact_hedera_payload_amount_mismatch"));
    }

    if get_positive_receivers(transfers)
        .iter()
        .any(|id| !hedera_account_ids_equal(id, pay_to))
    {
        return Err(invalid(
            "invalid_exact_hedera_payload_extra_positive_transfers",
        ));
    }
    Ok(())
}

fn is_valid_pay_to_format(pay_to: &str, alias_policy: AliasPolicy) -> bool {
    if pay_to.is_empty() || pay_to.chars().all(char::is_whitespace) {
        return false;
    }
    if alias_policy == AliasPolicy::Allow {
        return pay_to.parse::<crate::chain::HederaAddress>().is_ok();
    }
    is_entity_id(pay_to)
}

async fn validate_pay_to_policy<P: HederaFacilitatorOps>(
    provider: &P,
    alias_policy: AliasPolicy,
    pay_to: &str,
    network: &str,
) -> Result<(), HederaInvalid> {
    if alias_policy == AliasPolicy::Allow {
        return Ok(());
    }
    if !is_entity_id(pay_to) {
        return Err(invalid(
            "invalid_exact_hedera_payload_pay_to_alias_not_allowed",
        ));
    }
    let resolved = provider
        .resolve_account(pay_to, network)
        .await
        .map_err(|_| invalid("invalid_exact_hedera_payload_pay_to_alias_not_allowed"))?;
    if !resolved.exists || resolved.is_alias {
        return Err(invalid(
            "invalid_exact_hedera_payload_pay_to_alias_not_allowed",
        ));
    }
    Ok(())
}
