//! Facilitator verification for the Algorand exact scheme.

use base64::Engine;
use compact_str::CompactString;
use r402_core::wire::VerifyResponse;
use serde_json::Value;

use crate::MAX_TRANSACTION_GROUP_SIZE;
use crate::chain::codec::{SignedTransaction, Transaction, TxnType, decode_signed};
use crate::chain::rpc::AlgodRpc;
use crate::chain::signer::verify_txn_signature;
use crate::chain::types::{AlgorandAddress, normalize_algorand_network};
use crate::exact::error::{AlgorandInvalid, invalid};
use crate::max_reasonable_group_fee;

/// Verifies a raw facilitator verify/settle request JSON.
pub async fn verify_request_json<R, F>(
    rpc: &R,
    facilitator_addresses: &[String],
    request: &Value,
    sign: F,
) -> VerifyResponse
where
    R: AlgodRpc,
    F: FnMut(&Transaction, &str) -> Result<Vec<u8>, AlgorandInvalid>,
{
    match verify_inner(rpc, facilitator_addresses, request, sign).await {
        Ok(payer) => VerifyResponse::valid(payer),
        Err(invalid) => invalid.into_response(),
    }
}

#[allow(
    clippy::too_many_lines,
    clippy::cognitive_complexity,
    reason = "sequential AVM verify checklist stays in one function"
)]
async fn verify_inner<R, F>(
    rpc: &R,
    facilitator_addresses: &[String],
    request: &Value,
    mut sign: F,
) -> Result<CompactString, AlgorandInvalid>
where
    R: AlgodRpc,
    F: FnMut(&Transaction, &str) -> Result<Vec<u8>, AlgorandInvalid>,
{
    let payload = request
        .get("paymentPayload")
        .ok_or_else(|| invalid("invalid_exact_avm_payload"))?;
    let requirements = request
        .get("paymentRequirements")
        .ok_or_else(|| invalid("invalid_exact_avm_payload"))?;

    let payload_version = payload.get("x402Version").and_then(Value::as_u64);
    if payload_version != Some(2) {
        return Err(invalid("invalid_exact_avm_invalid_version")
            .with_message(format!("Expected x402Version 2, got {payload_version:?}")));
    }

    let accepted = payload
        .get("accepted")
        .ok_or_else(|| invalid("invalid_exact_avm_payload"))?;
    let accepted_scheme = accepted.get("scheme").and_then(Value::as_str);
    let req_scheme = requirements.get("scheme").and_then(Value::as_str);
    if accepted_scheme != Some("exact") || req_scheme != Some("exact") {
        return Err(invalid("invalid_exact_avm_scheme").with_message(format!(
            "Expected scheme \"exact\", got payload=\"{accepted_scheme:?}\" requirements=\"{req_scheme:?}\""
        )));
    }

    let req_network = requirements
        .get("network")
        .and_then(Value::as_str)
        .unwrap_or("");
    let accepted_network = accepted
        .get("network")
        .and_then(Value::as_str)
        .unwrap_or("");
    let req_chain = normalize_algorand_network(req_network)
        .ok_or_else(|| invalid("invalid_exact_avm_payload").with_message("unsupported network"))?;
    let accepted_chain = normalize_algorand_network(accepted_network).ok_or_else(|| {
        invalid("invalid_exact_avm_payload").with_message("unsupported payload network")
    })?;
    if req_chain != accepted_chain {
        return Err(
            invalid("invalid_exact_avm_network_mismatch").with_message(format!(
                "Network mismatch: payload=\"{accepted_network}\" requirements=\"{req_network}\""
            )),
        );
    }

    let inner = payload
        .get("payload")
        .ok_or_else(|| invalid("invalid_exact_avm_payload"))?;
    let payment_group = inner
        .get("paymentGroup")
        .and_then(Value::as_array)
        .ok_or_else(|| invalid("invalid_exact_avm_payload"))?;
    if !payment_group.iter().all(Value::is_string) {
        return Err(invalid("invalid_exact_avm_payload"));
    }
    let payment_index = inner
        .get("paymentIndex")
        .and_then(Value::as_u64)
        .ok_or_else(|| invalid("invalid_exact_avm_payload"))?;

    if payment_group.len() > MAX_TRANSACTION_GROUP_SIZE {
        return Err(
            invalid("invalid_exact_avm_group_size_exceeded").with_message(format!(
                "Transaction group has {} transactions, maximum is {MAX_TRANSACTION_GROUP_SIZE}",
                payment_group.len()
            )),
        );
    }

    let group_len = u64::try_from(payment_group.len()).unwrap_or(u64::MAX);
    if payment_index >= group_len {
        return Err(
            invalid("invalid_exact_avm_payment_index").with_message(format!(
                "Payment index {payment_index} out of bounds for group of {}",
                payment_group.len()
            )),
        );
    }

    let decoded = decode_transaction_group(payment_group, facilitator_addresses)?;
    let payment_idx = usize::try_from(payment_index).unwrap_or(usize::MAX);
    let payment = decoded
        .get(payment_idx)
        .ok_or_else(|| invalid("invalid_exact_avm_payment_index"))?;
    let payer = payment.txn.sender.to_string();

    if facilitator_addresses.iter().any(|a| a == &payer) {
        return Err(invalid("invalid_exact_avm_facilitator_transferring")
            .with_payer(payer)
            .with_message("Facilitator signer cannot be the payment sender"));
    }

    let encoded_payment = payment_group
        .get(payment_idx)
        .and_then(Value::as_str)
        .ok_or_else(|| invalid("invalid_exact_avm_payload"))?;
    verify_payment_transaction(payment, requirements, encoded_payment)?;

    let prepared =
        prepare_signed_group_with(&decoded, payment_group, facilitator_addresses, &mut sign)?;
    let sim = rpc
        .simulate_group(&prepared)
        .await
        .map_err(|e| invalid("invalid_exact_avm_simulation_failed").with_message(e.to_string()))?;
    if let Some(message) = sim.failure_message {
        return Err(invalid("invalid_exact_avm_simulation_failed")
            .with_payer(payer)
            .with_message(message));
    }

    Ok(CompactString::from(payer))
}

/// Decodes every group element; unsigned txns must come from a facilitator.
pub(crate) fn decode_transaction_group(
    payment_group: &[Value],
    facilitator_addresses: &[String],
) -> Result<Vec<SignedTransaction>, AlgorandInvalid> {
    let mut txns = Vec::with_capacity(payment_group.len());
    for (i, item) in payment_group.iter().enumerate() {
        let encoded = item
            .as_str()
            .ok_or_else(|| invalid("invalid_exact_avm_payload"))?;
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(encoded)
            .map_err(|_| {
                invalid("invalid_exact_avm_invalid_transaction")
                    .with_message(format!("Failed to decode transaction at index {i}"))
            })?;
        let stxn = decode_signed(&bytes).map_err(|_| {
            invalid("invalid_exact_avm_invalid_transaction")
                .with_message(format!("Failed to decode transaction at index {i}"))
        })?;
        if !stxn.has_signature() {
            let sender = stxn.txn.sender.to_string();
            if !facilitator_addresses.iter().any(|a| a == &sender) {
                return Err(invalid("invalid_exact_avm_unsigned_non_facilitator")
                    .with_message(format!(
                        "Unsigned transaction at index {i} from {sender} is not a facilitator address"
                    )));
            }
        }
        txns.push(stxn);
    }

    if txns.len() > 1 {
        let first = txns.first().and_then(|t| t.txn.group);
        for txn in txns.iter().skip(1) {
            if txn.txn.group != first {
                return Err(invalid("invalid_exact_avm_invalid_group_id")
                    .with_message("Transactions have inconsistent group IDs"));
            }
        }
    }

    Ok(txns)
}

fn verify_payment_transaction(
    stxn: &SignedTransaction,
    requirements: &Value,
    _encoded: &str,
) -> Result<(), AlgorandInvalid> {
    if stxn.txn.txn_type != TxnType::Axfer {
        return Err(
            invalid("invalid_exact_avm_not_asset_transfer").with_message(format!(
                "Expected asset transfer, got \"{}\"",
                stxn.txn.txn_type.as_str()
            )),
        );
    }

    let expected_amount = requirements
        .get("amount")
        .and_then(Value::as_str)
        .unwrap_or("");
    let actual = stxn.txn.asset_amount.to_string();
    if actual != expected_amount {
        return Err(invalid("invalid_exact_avm_amount_mismatch")
            .with_message(format!("Expected {expected_amount}, got {actual}")));
    }

    let expected_pay_to = requirements
        .get("payTo")
        .and_then(Value::as_str)
        .unwrap_or("");
    let receiver = stxn
        .txn
        .asset_receiver
        .unwrap_or(AlgorandAddress::ZERO)
        .to_string();
    if receiver != expected_pay_to {
        return Err(invalid("invalid_exact_avm_receiver_mismatch")
            .with_message(format!("Expected {expected_pay_to}, got {receiver}")));
    }

    let expected_asset = requirements
        .get("asset")
        .and_then(Value::as_str)
        .unwrap_or("");
    let asset_id = stxn.txn.asset_id.to_string();
    if asset_id != expected_asset {
        return Err(invalid("invalid_exact_avm_asset_mismatch")
            .with_message(format!("Expected asset {expected_asset}, got {asset_id}")));
    }

    let Some(sig) = stxn.sig else {
        return Err(invalid("invalid_exact_avm_payment_not_signed")
            .with_message("Payment transaction is not signed"));
    };
    if !verify_txn_signature(&stxn.txn, &sig) {
        return Err(invalid("invalid_exact_avm_invalid_signature")
            .with_message("Payment transaction signature does not match sender"));
    }
    Ok(())
}

fn verify_fee_payer_transaction(
    txn: &Transaction,
    group_size: usize,
) -> Result<(), AlgorandInvalid> {
    if txn.txn_type != TxnType::Pay {
        return Err(
            invalid("invalid_exact_avm_invalid_fee_payer").with_message(format!(
                "Expected payment transaction, got {}",
                txn.txn_type.as_str()
            )),
        );
    }
    if txn.amount > 0 {
        return Err(invalid("invalid_exact_avm_invalid_fee_payer")
            .with_message("Fee payer amount must be 0"));
    }
    if let Some(receiver) = txn.receiver
        && receiver != txn.sender
    {
        return Err(invalid("invalid_exact_avm_invalid_fee_payer")
            .with_message("Fee payer receiver must be same as sender (self-payment)"));
    }
    if txn.close_remainder_to.is_some() {
        return Err(invalid("invalid_exact_avm_invalid_fee_payer")
            .with_message("closeRemainderTo not allowed on fee payer"));
    }
    if txn.rekey_to.is_some() {
        return Err(invalid("invalid_exact_avm_invalid_fee_payer")
            .with_message("rekeyTo not allowed on fee payer"));
    }
    let max_fee = max_reasonable_group_fee(group_size);
    if txn.fee > max_fee {
        return Err(
            invalid("invalid_exact_avm_fee_too_high").with_message(format!(
                "Fee {} exceeds maximum {max_fee} ({group_size} txns × 5000 µAlgo)",
                txn.fee
            )),
        );
    }
    Ok(())
}

/// Signs facilitator-owned txns and returns the assembled group bytes.
pub(crate) fn prepare_signed_group_with<F>(
    decoded: &[SignedTransaction],
    payment_group: &[Value],
    facilitator_addresses: &[String],
    sign: &mut F,
) -> Result<Vec<Vec<u8>>, AlgorandInvalid>
where
    F: FnMut(&Transaction, &str) -> Result<Vec<u8>, AlgorandInvalid>,
{
    let mut signed = Vec::with_capacity(decoded.len());
    for (i, stxn) in decoded.iter().enumerate() {
        let sender = stxn.txn.sender.to_string();
        if facilitator_addresses.iter().any(|a| a == &sender) {
            verify_fee_payer_transaction(&stxn.txn, decoded.len())?;
            signed.push(sign(&stxn.txn, &sender)?);
        } else {
            let encoded = payment_group
                .get(i)
                .and_then(Value::as_str)
                .ok_or_else(|| invalid("invalid_exact_avm_payload"))?;
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(encoded)
                .map_err(|_| invalid("invalid_exact_avm_invalid_transaction"))?;
            signed.push(bytes);
        }
    }
    Ok(signed)
}
