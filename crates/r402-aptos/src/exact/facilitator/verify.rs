//! Facilitator verification for the Aptos exact scheme.

use aptos_sdk::aptos_bcs;
use aptos_sdk::transaction::authenticator::AccountAuthenticator;
use aptos_sdk::types::AccountAddress;
use compact_str::CompactString;
use r402_core::wire::VerifyResponse;
use serde_json::Value;

use super::AptosFacilitatorOps;
use crate::chain::codec::{DecodedAptosPayment, is_metadata_type_tag, is_supported_transfer};
use crate::chain::types::is_aptos_network;
use crate::exact::error::{AptosInvalid, invalid};
use crate::{EXPIRATION_BUFFER_SECONDS, MAX_GAS_AMOUNT, MAX_GAS_UNIT_PRICE};

/// Verifies a raw facilitator verify/settle request JSON.
pub async fn verify_request_json<P: AptosFacilitatorOps>(
    provider: &P,
    request: &Value,
) -> VerifyResponse {
    match verify_inner(provider, request).await {
        Ok(payer) => VerifyResponse::valid(payer),
        Err(invalid) => invalid.into_response(),
    }
}

#[allow(
    clippy::too_many_lines,
    clippy::cognitive_complexity,
    reason = "sequential TS verify checklist must stay in one function"
)]
async fn verify_inner<P: AptosFacilitatorOps>(
    provider: &P,
    request: &Value,
) -> Result<CompactString, AptosInvalid> {
    let payload = request
        .get("paymentPayload")
        .ok_or_else(|| invalid("invalid_payload"))?;
    let requirements = request
        .get("paymentRequirements")
        .ok_or_else(|| invalid("invalid_payment_requirements"))?;

    if payload.get("x402Version").and_then(Value::as_u64) != Some(2) {
        return Err(invalid("invalid_exact_aptos_payload_unsupported_version"));
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
    if !is_aptos_network(req_network) {
        return Err(invalid("network_mismatch"));
    }

    let expected_chain_id = match req_network {
        "aptos:1" => 1u8,
        "aptos:2" => 2u8,
        _ => return Err(invalid("network_mismatch")),
    };

    let fee_payer = requirements
        .get("extra")
        .and_then(|e| e.get("feePayer"))
        .and_then(Value::as_str);
    let is_sponsored = fee_payer.is_some();
    let managed = provider.fee_payer_addresses();

    if let Some(fee_payer) = fee_payer {
        let expected = parse_address(fee_payer)
            .ok_or_else(|| invalid("fee_payer_not_managed_by_facilitator"))?;
        if !managed
            .iter()
            .any(|id| parse_address(id).is_some_and(|addr| addr == expected))
        {
            return Err(invalid("fee_payer_not_managed_by_facilitator"));
        }
    }

    let transaction_b64 = payload
        .get("payload")
        .and_then(|p| p.get("transaction"))
        .and_then(Value::as_str)
        .unwrap_or("");
    if transaction_b64.is_empty() {
        return Err(invalid("invalid_exact_aptos_payload_verification_error"));
    }

    let decoded = DecodedAptosPayment::from_base64(transaction_b64).map_err(|e| {
        AptosInvalid::from_owned(format!(
            "invalid_exact_aptos_payload_verification_error: {e}"
        ))
    })?;
    let sender = CompactString::from(decoded.sender_long());
    let raw = &decoded.transaction.raw_transaction;

    let tx_chain_id = raw.chain_id.id();
    if tx_chain_id != expected_chain_id {
        return Err(AptosInvalid::from_owned(format!(
            "invalid_exact_aptos_payload_chain_id_mismatch: expected {expected_chain_id}, got {tx_chain_id}"
        ))
        .with_payer(sender));
    }

    let signing_message = decoded.transaction.signing_message().map_err(|e| {
        AptosInvalid::from_owned(format!(
            "invalid_exact_aptos_payload_verification_error: {e}"
        ))
        .with_payer(sender.clone())
    })?;

    match &decoded.sender_authenticator {
        AccountAuthenticator::Ed25519 { .. } => {
            match decoded.sender_authenticator.derived_address() {
                Ok(derived) if derived == raw.sender => {}
                _ => {
                    return Err(invalid(
                        "invalid_exact_aptos_payload_sender_authenticator_mismatch",
                    )
                    .with_payer(sender));
                }
            }
            if decoded
                .sender_authenticator
                .verify(&signing_message)
                .is_err()
            {
                return Err(invalid(
                    "invalid_exact_aptos_payload_sender_authenticator_invalid_signature",
                )
                .with_payer(sender));
            }
        }
        AccountAuthenticator::SingleKey { .. } | AccountAuthenticator::MultiKey { .. } => {
            if decoded
                .sender_authenticator
                .verify(&signing_message)
                .is_err()
            {
                return Err(invalid(
                    "invalid_exact_aptos_payload_sender_authenticator_invalid_signature",
                )
                .with_payer(sender));
            }
        }
        AccountAuthenticator::MultiEd25519 { .. }
        | AccountAuthenticator::NoAccountAuthenticator => {
            return Err(
                invalid("invalid_exact_aptos_payload_unsupported_authenticator").with_payer(sender),
            );
        }
    }

    if is_sponsored {
        if raw.max_gas_amount > MAX_GAS_AMOUNT {
            return Err(AptosInvalid::from_owned(format!(
                "invalid_exact_aptos_payload_gas_too_high: {} > {MAX_GAS_AMOUNT}",
                raw.max_gas_amount
            ))
            .with_payer(sender));
        }
        if raw.gas_unit_price > MAX_GAS_UNIT_PRICE {
            return Err(AptosInvalid::from_owned(format!(
                "invalid_exact_aptos_payload_gas_unit_price_too_high: {} > {MAX_GAS_UNIT_PRICE}",
                raw.gas_unit_price
            ))
            .with_payer(sender));
        }

        let expected = fee_payer.and_then(parse_address).ok_or_else(|| {
            invalid("invalid_exact_aptos_payload_fee_payer_mismatch").with_payer(sender.clone())
        })?;
        if decoded.transaction.fee_payer_address != Some(expected) {
            return Err(
                invalid("invalid_exact_aptos_payload_fee_payer_mismatch").with_payer(sender)
            );
        }

        if managed
            .iter()
            .any(|id| parse_address(id).is_some_and(|addr| addr == raw.sender))
        {
            return Err(
                invalid("invalid_exact_aptos_payload_fee_payer_transferring_funds")
                    .with_payer(sender),
            );
        }
    }

    let now = now_secs();
    if raw.expiration_timestamp_secs < now.saturating_add(EXPIRATION_BUFFER_SECONDS) {
        return Err(invalid("invalid_exact_aptos_payload_transaction_expired").with_payer(sender));
    }

    let Some(entry) = decoded.entry_function() else {
        return Err(
            invalid("invalid_exact_aptos_payload_missing_entry_function").with_payer(sender),
        );
    };
    if !is_supported_transfer(entry) {
        return Err(invalid("invalid_exact_aptos_payload_wrong_function").with_payer(sender));
    }
    if entry.type_args.len() != 1 || !entry.type_args.first().is_some_and(is_metadata_type_tag) {
        return Err(invalid("invalid_exact_aptos_payload_wrong_type_args").with_payer(sender));
    }
    if entry.args.len() != 3 {
        return Err(invalid("invalid_exact_aptos_payload_wrong_args").with_payer(sender));
    }

    let req_asset = requirements
        .get("asset")
        .and_then(Value::as_str)
        .unwrap_or("");
    let req_amount = requirements
        .get("amount")
        .and_then(Value::as_str)
        .unwrap_or("");
    let req_pay_to = requirements
        .get("payTo")
        .and_then(Value::as_str)
        .unwrap_or("");

    let fa =
        decode_address_arg(entry.args.first().map_or(&[][..], Vec::as_slice)).ok_or_else(|| {
            invalid("invalid_exact_aptos_payload_asset_mismatch").with_payer(sender.clone())
        })?;
    let expected_asset = parse_address(req_asset).ok_or_else(|| {
        invalid("invalid_exact_aptos_payload_asset_mismatch").with_payer(sender.clone())
    })?;
    if fa != expected_asset {
        return Err(invalid("invalid_exact_aptos_payload_asset_mismatch").with_payer(sender));
    }

    let amount =
        decode_u64_arg(entry.args.get(2).map_or(&[][..], Vec::as_slice)).ok_or_else(|| {
            invalid("invalid_exact_aptos_payload_amount_mismatch").with_payer(sender.clone())
        })?;
    if amount.to_string() != req_amount {
        return Err(invalid("invalid_exact_aptos_payload_amount_mismatch").with_payer(sender));
    }

    let recipient = decode_address_arg(entry.args.get(1).map_or(&[][..], Vec::as_slice))
        .ok_or_else(|| {
            invalid("invalid_exact_aptos_payload_recipient_mismatch").with_payer(sender.clone())
        })?;
    let expected_pay_to = parse_address(req_pay_to).ok_or_else(|| {
        invalid("invalid_exact_aptos_payload_recipient_mismatch").with_payer(sender.clone())
    })?;
    if recipient != expected_pay_to {
        return Err(invalid("invalid_exact_aptos_payload_recipient_mismatch").with_payer(sender));
    }

    let req_amount_u64: u64 = req_amount.parse().map_err(|_| {
        invalid("invalid_exact_aptos_payload_amount_mismatch").with_payer(sender.clone())
    })?;
    let balance = provider
        .fungible_asset_balance(&sender, req_asset, req_network)
        .await
        .map_err(|e| {
            AptosInvalid::from_owned(format!(
                "invalid_exact_aptos_payload_verification_error: {e}"
            ))
            .with_payer(sender.clone())
        })?;
    if balance < req_amount_u64 {
        return Err(invalid("invalid_exact_aptos_payload_insufficient_balance").with_payer(sender));
    }

    let simulation = provider
        .simulate_payment(transaction_b64, req_network)
        .await
        .map_err(|e| {
            AptosInvalid::from_owned(format!(
                "invalid_exact_aptos_payload_verification_error: {e}"
            ))
            .with_payer(sender.clone())
        })?;
    if !simulation.success {
        return Err(AptosInvalid::from_owned(format!(
            "invalid_exact_aptos_payload_simulation_failed: {}",
            simulation.vm_status
        ))
        .with_payer(sender));
    }

    Ok(sender)
}

fn parse_address(s: &str) -> Option<AccountAddress> {
    AccountAddress::from_hex(s).ok()
}

fn decode_address_arg(bytes: &[u8]) -> Option<AccountAddress> {
    aptos_bcs::from_bytes(bytes).ok()
}

fn decode_u64_arg(bytes: &[u8]) -> Option<u64> {
    aptos_bcs::from_bytes(bytes).ok()
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}
