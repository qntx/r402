//! Facilitator verification for the TON exact scheme.

use crate::chain::rpc::TvmRpc;
use crate::chain::{TvmAddress, TvmRelayRequest};
use crate::codecs::common::cell_hash_hex;
use crate::codecs::w5::{
    ParsedTvmSettlement, StateInitCells, address_from_state_init, is_allowed_client_code,
    parse_exact_tvm_payload, parse_w5_init_data,
};
use crate::exact::error::{
    ERR_EXACT_TVM_ACCOUNT_FROZEN, ERR_EXACT_TVM_FACILITATOR_INSUFFICIENT_BALANCE,
    ERR_EXACT_TVM_INSUFFICIENT_BALANCE, ERR_EXACT_TVM_INVALID_AMOUNT, ERR_EXACT_TVM_INVALID_ASSET,
    ERR_EXACT_TVM_INVALID_CODE_HASH, ERR_EXACT_TVM_INVALID_EXTENSIONS_DICT,
    ERR_EXACT_TVM_INVALID_JETTON_TRANSFER, ERR_EXACT_TVM_INVALID_PAYLOAD,
    ERR_EXACT_TVM_INVALID_RECIPIENT, ERR_EXACT_TVM_INVALID_SEQNO, ERR_EXACT_TVM_INVALID_SIGNATURE,
    ERR_EXACT_TVM_INVALID_SIGNATURE_MODE, ERR_EXACT_TVM_INVALID_UNTIL_EXPIRED,
    ERR_EXACT_TVM_INVALID_W5_MESSAGE, ERR_EXACT_TVM_INVALID_WALLET_ID,
    ERR_EXACT_TVM_NETWORK_MISMATCH, ERR_EXACT_TVM_SIMULATION_FAILED,
    ERR_EXACT_TVM_TON_AMOUNT_TOO_HIGH, ERR_EXACT_TVM_UNSUPPORTED_NETWORK,
    ERR_EXACT_TVM_UNSUPPORTED_SCHEME, ERR_EXACT_TVM_UNSUPPORTED_VERSION,
    ERR_EXACT_TVM_VALID_UNTIL_TOO_FAR, TvmInvalid, invalid,
};
use crate::exact::types::TvmExtra;
use crate::trace::{
    message_body_hash_matches, normalize_address_or_null, parse_trace_transactions,
    trace_transaction_compute_fees, trace_transaction_fwd_fees, trace_transaction_hash_to_hex,
    trace_transaction_storage_fees, transaction_succeeded,
};
use crate::{
    DEFAULT_JETTON_WALLET_MESSAGE_AMOUNT, DEFAULT_MAX_TIMEOUT_SECONDS,
    DEFAULT_TVM_OUTER_GAS_BUFFER, MIN_FACILITATOR_TON_BALANCE, W5R1_CODE_HASH,
};
use compact_str::CompactString;
use r402_core::wire::VerifyResponse;
use serde_json::Value;

/// Successful verify plus the relay request used for settle.
#[derive(Debug, Clone)]
pub struct VerifiedSettlement {
    /// Payer raw address.
    pub payer: CompactString,
    /// Parsed settlement.
    pub settlement: ParsedTvmSettlement,
    /// Relay request with estimated outer TON.
    pub relay_request: TvmRelayRequest,
}

/// Verifies a raw facilitator verify/settle request JSON.
pub async fn verify_request_json<R: TvmRpc>(
    rpc: &R,
    facilitator_addresses: &[String],
    request: &Value,
    now_unix: u64,
    emulate: impl AsyncFn(&TvmRelayRequest) -> Result<Value, String>,
) -> Result<VerifiedSettlement, TvmInvalid> {
    verify_inner(rpc, facilitator_addresses, request, now_unix, emulate).await
}

/// Convenience wrapper that returns a wire [`VerifyResponse`].
pub async fn verify_response_json<R: TvmRpc>(
    rpc: &R,
    facilitator_addresses: &[String],
    request: &Value,
    now_unix: u64,
    emulate: impl AsyncFn(&TvmRelayRequest) -> Result<Value, String>,
) -> VerifyResponse {
    match verify_request_json(rpc, facilitator_addresses, request, now_unix, emulate).await {
        Ok(verified) => VerifyResponse::valid(verified.payer),
        Err(invalid) => invalid.into_response(),
    }
}

#[allow(
    clippy::too_many_lines,
    clippy::cognitive_complexity,
    reason = "sequential TS verify checklist must stay in one function"
)]
async fn verify_inner<R: TvmRpc>(
    rpc: &R,
    facilitator_addresses: &[String],
    request: &Value,
    now_unix: u64,
    emulate: impl AsyncFn(&TvmRelayRequest) -> Result<Value, String>,
) -> Result<VerifiedSettlement, TvmInvalid> {
    let payload = request
        .get("paymentPayload")
        .ok_or_else(|| invalid("invalid_payload"))?;
    let requirements = request
        .get("paymentRequirements")
        .ok_or_else(|| invalid("invalid_payment_requirements"))?;

    if payload.get("x402Version").and_then(Value::as_u64) != Some(2) {
        return Err(invalid(ERR_EXACT_TVM_UNSUPPORTED_VERSION));
    }
    let accepted = payload
        .get("accepted")
        .ok_or_else(|| invalid(ERR_EXACT_TVM_INVALID_PAYLOAD))?;
    if accepted.get("scheme").and_then(Value::as_str) != Some("exact")
        || requirements.get("scheme").and_then(Value::as_str) != Some("exact")
    {
        return Err(invalid(ERR_EXACT_TVM_UNSUPPORTED_SCHEME));
    }
    let req_network = requirements
        .get("network")
        .and_then(Value::as_str)
        .unwrap_or("");
    if !crate::chain::is_tvm_network(req_network) {
        return Err(invalid(ERR_EXACT_TVM_UNSUPPORTED_NETWORK));
    }
    if accepted
        .get("network")
        .and_then(Value::as_str)
        .unwrap_or("")
        != req_network
    {
        return Err(invalid(ERR_EXACT_TVM_NETWORK_MISMATCH));
    }

    for addr in facilitator_addresses {
        let state = rpc.get_account_state(addr).await.map_err(|e| {
            invalid(ERR_EXACT_TVM_FACILITATOR_INSUFFICIENT_BALANCE).with_message(e.to_string())
        })?;
        if state.balance < MIN_FACILITATOR_TON_BALANCE {
            return Err(invalid(ERR_EXACT_TVM_FACILITATOR_INSUFFICIENT_BALANCE).with_message(
                format!(
                    "Facilitator wallet {addr} balance {} nanotons is below required {MIN_FACILITATOR_TON_BALANCE} nanotons",
                    state.balance
                ),
            ));
        }
    }

    if json_u128(accepted.get("amount")) != json_u128(requirements.get("amount")) {
        return Err(invalid(ERR_EXACT_TVM_INVALID_AMOUNT));
    }
    if normalize_json_address(accepted.get("asset"))
        != normalize_json_address(requirements.get("asset"))
    {
        return Err(invalid(ERR_EXACT_TVM_INVALID_ASSET));
    }
    if normalize_json_address(accepted.get("payTo"))
        != normalize_json_address(requirements.get("payTo"))
    {
        return Err(invalid(ERR_EXACT_TVM_INVALID_RECIPIENT));
    }

    let accepted_extra = extra_from_json(accepted.get("extra"))?;
    let requirements_extra = extra_from_json(requirements.get("extra"))?;
    if !accepted_extra.are_fees_sponsored || !requirements_extra.are_fees_sponsored {
        return Err(invalid(ERR_EXACT_TVM_UNSUPPORTED_SCHEME));
    }

    let inner = payload
        .get("payload")
        .ok_or_else(|| invalid(ERR_EXACT_TVM_INVALID_PAYLOAD))?;
    let settlement_boc = inner
        .get("settlementBoc")
        .and_then(Value::as_str)
        .filter(|s| !s.trim().is_empty())
        .ok_or_else(|| invalid(ERR_EXACT_TVM_INVALID_PAYLOAD))?;
    let payload_asset = inner
        .get("asset")
        .and_then(Value::as_str)
        .ok_or_else(|| invalid(ERR_EXACT_TVM_INVALID_PAYLOAD))?;

    let settlement = parse_exact_tvm_payload(settlement_boc)
        .map_err(|e| TvmInvalid::from_reason(e.to_string()))?;
    let payer = CompactString::from(settlement.payer.as_str());

    let req_asset = normalize_json_address(requirements.get("asset"))
        .ok_or_else(|| invalid(ERR_EXACT_TVM_INVALID_ASSET).with_payer(payer.clone()))?;
    let parsed_asset: TvmAddress = payload_asset
        .parse()
        .map_err(|_| invalid(ERR_EXACT_TVM_INVALID_ASSET).with_payer(payer.clone()))?;
    if parsed_asset != req_asset {
        return Err(invalid(ERR_EXACT_TVM_INVALID_ASSET).with_payer(payer));
    }

    let expected_response = requirements_extra.effective_response_destination().cloned();
    if accepted_extra.effective_response_destination() != expected_response.as_ref() {
        return Err(invalid(ERR_EXACT_TVM_INVALID_JETTON_TRANSFER).with_payer(payer));
    }
    let expected_fwd = requirements_extra
        .forward_ton_amount_u128()
        .map_err(|_| invalid(ERR_EXACT_TVM_INVALID_JETTON_TRANSFER).with_payer(payer.clone()))?;
    let accepted_fwd = accepted_extra
        .forward_ton_amount_u128()
        .map_err(|_| invalid(ERR_EXACT_TVM_INVALID_JETTON_TRANSFER).with_payer(payer.clone()))?;
    if accepted_fwd != expected_fwd {
        return Err(invalid(ERR_EXACT_TVM_INVALID_JETTON_TRANSFER).with_payer(payer));
    }
    let expected_payload = requirements_extra
        .effective_forward_payload()
        .map_err(|_| invalid(ERR_EXACT_TVM_INVALID_JETTON_TRANSFER).with_payer(payer.clone()))?;
    let accepted_payload = accepted_extra
        .effective_forward_payload()
        .map_err(|_| invalid(ERR_EXACT_TVM_INVALID_JETTON_TRANSFER).with_payer(payer.clone()))?;
    if cell_hash_hex(&accepted_payload) != cell_hash_hex(&expected_payload) {
        return Err(invalid(ERR_EXACT_TVM_INVALID_JETTON_TRANSFER).with_payer(payer));
    }

    let req_pay_to = normalize_json_address(requirements.get("payTo"))
        .ok_or_else(|| invalid(ERR_EXACT_TVM_INVALID_RECIPIENT).with_payer(payer.clone()))?;
    if settlement.transfer.destination != req_pay_to {
        return Err(invalid(ERR_EXACT_TVM_INVALID_RECIPIENT).with_payer(payer));
    }
    let req_amount = json_u128(requirements.get("amount"))
        .ok_or_else(|| invalid(ERR_EXACT_TVM_INVALID_AMOUNT).with_payer(payer.clone()))?;
    if settlement.transfer.jetton_amount != req_amount {
        return Err(invalid(ERR_EXACT_TVM_INVALID_AMOUNT).with_payer(payer));
    }
    if settlement.transfer.forward_ton_amount != expected_fwd {
        return Err(invalid(ERR_EXACT_TVM_INVALID_JETTON_TRANSFER).with_payer(payer));
    }
    if settlement.transfer.response_destination != expected_response {
        return Err(invalid(ERR_EXACT_TVM_INVALID_JETTON_TRANSFER).with_payer(payer));
    }
    if cell_hash_hex(&settlement.transfer.forward_payload) != cell_hash_hex(&expected_payload) {
        return Err(invalid(ERR_EXACT_TVM_INVALID_JETTON_TRANSFER).with_payer(payer));
    }
    let max_attached = settlement
        .transfer
        .forward_ton_amount
        .saturating_add(DEFAULT_JETTON_WALLET_MESSAGE_AMOUNT);
    if settlement.transfer.attached_ton_amount > max_attached {
        return Err(invalid(ERR_EXACT_TVM_TON_AMOUNT_TOO_HIGH).with_payer(payer));
    }

    if u64::from(settlement.valid_until) <= now_unix {
        return Err(invalid(ERR_EXACT_TVM_INVALID_UNTIL_EXPIRED).with_payer(payer));
    }
    let max_timeout = requirements
        .get("maxTimeoutSeconds")
        .and_then(Value::as_u64)
        .unwrap_or(DEFAULT_MAX_TIMEOUT_SECONDS);
    if u64::from(settlement.valid_until) > now_unix.saturating_add(max_timeout) {
        return Err(invalid(ERR_EXACT_TVM_VALID_UNTIL_TOO_FAR).with_payer(payer));
    }

    let account = rpc
        .get_account_state(settlement.payer.as_str())
        .await
        .map_err(|e| {
            invalid(ERR_EXACT_TVM_SIMULATION_FAILED)
                .with_payer(payer.clone())
                .with_message(e.to_string())
        })?;
    if account.is_frozen {
        return Err(invalid(ERR_EXACT_TVM_ACCOUNT_FROZEN).with_payer(payer));
    }

    let init_data =
        if let (Some(state_init), true) = (&settlement.state_init, account.is_uninitialized) {
            if !is_allowed_client_code(&state_init.code_hash_hex()) {
                return Err(invalid(ERR_EXACT_TVM_INVALID_CODE_HASH).with_payer(payer));
            }
            let derived = address_from_state_init(state_init, 0)
                .map_err(|_| invalid(ERR_EXACT_TVM_INVALID_W5_MESSAGE).with_payer(payer.clone()))?;
            if derived != settlement.payer {
                return Err(invalid(ERR_EXACT_TVM_INVALID_W5_MESSAGE).with_payer(payer));
            }
            let parsed = parse_w5_init_data(state_init)
                .map_err(|_| invalid(ERR_EXACT_TVM_INVALID_W5_MESSAGE).with_payer(payer.clone()))?;
            if parsed.seqno != 0 {
                return Err(invalid(ERR_EXACT_TVM_INVALID_SEQNO).with_payer(payer));
            }
            if parsed.has_extensions {
                return Err(invalid(ERR_EXACT_TVM_INVALID_EXTENSIONS_DICT).with_payer(payer));
            }
            parsed
        } else {
            let Some(state_init) = account.state_init.as_ref() else {
                return Err(invalid(ERR_EXACT_TVM_INVALID_CODE_HASH).with_payer(payer));
            };
            if !is_allowed_client_code(&state_init.code_hash_hex()) {
                return Err(invalid(ERR_EXACT_TVM_INVALID_CODE_HASH).with_payer(payer));
            }
            parse_w5_init_data(state_init)
                .map_err(|_| invalid(ERR_EXACT_TVM_INVALID_CODE_HASH).with_payer(payer.clone()))?
        };

    if !init_data.signature_allowed {
        return Err(invalid(ERR_EXACT_TVM_INVALID_SIGNATURE_MODE).with_payer(payer));
    }
    if init_data.seqno != settlement.seqno {
        return Err(invalid(ERR_EXACT_TVM_INVALID_SEQNO).with_payer(payer));
    }
    if init_data.wallet_id != settlement.wallet_id {
        return Err(invalid(ERR_EXACT_TVM_INVALID_WALLET_ID).with_payer(payer));
    }
    if !verify_ed25519(
        &init_data.public_key,
        settlement.signed_slice_hash.as_slice(),
        &settlement.signature,
    ) {
        return Err(invalid(ERR_EXACT_TVM_INVALID_SIGNATURE).with_payer(payer));
    }

    let canonical_source = rpc
        .get_jetton_wallet(req_asset.as_str(), settlement.payer.as_str())
        .await
        .map_err(|e| {
            invalid(ERR_EXACT_TVM_INVALID_JETTON_TRANSFER)
                .with_payer(payer.clone())
                .with_message(e.to_string())
        })?;
    if settlement.transfer.source_wallet != canonical_source {
        return Err(invalid(ERR_EXACT_TVM_INVALID_JETTON_TRANSFER).with_payer(payer));
    }
    let jetton = rpc
        .get_jetton_wallet_data(settlement.transfer.source_wallet.as_str())
        .await
        .map_err(|e| {
            invalid(ERR_EXACT_TVM_INSUFFICIENT_BALANCE)
                .with_payer(payer.clone())
                .with_message(e.to_string())
        })?;
    if jetton.owner != settlement.payer {
        return Err(invalid(ERR_EXACT_TVM_INVALID_RECIPIENT).with_payer(payer));
    }
    if jetton.jetton_minter != req_asset {
        return Err(invalid(ERR_EXACT_TVM_INVALID_ASSET).with_payer(payer));
    }
    if jetton.balance < settlement.transfer.jetton_amount {
        return Err(invalid(ERR_EXACT_TVM_INSUFFICIENT_BALANCE).with_payer(payer));
    }

    let provisional = TvmRelayRequest {
        destination: settlement.payer.clone(),
        body: settlement.body.clone(),
        state_init: settlement.state_init.clone(),
        forward_ton_amount: settlement.transfer.forward_ton_amount,
        relay_amount: None,
    };
    let emulation = emulate(&provisional).await.map_err(|e| {
        invalid(ERR_EXACT_TVM_SIMULATION_FAILED)
            .with_payer(payer.clone())
            .with_message(e)
    })?;
    let payer_tx =
        verify_finalized_trace_settlement(&emulation, &settlement, true).map_err(|e| {
            invalid(ERR_EXACT_TVM_SIMULATION_FAILED)
                .with_payer(payer.clone())
                .with_message(e)
        })?;
    let required_outer = settlement
        .transfer
        .attached_ton_amount
        .saturating_add(trace_transaction_storage_fees(&payer_tx))
        .saturating_add(trace_transaction_compute_fees(&payer_tx))
        .saturating_add(trace_transaction_fwd_fees(&payer_tx, 1))
        .saturating_add(DEFAULT_TVM_OUTER_GAS_BUFFER);

    Ok(VerifiedSettlement {
        payer,
        settlement: settlement.clone(),
        relay_request: TvmRelayRequest {
            destination: settlement.payer,
            body: settlement.body,
            state_init: settlement.state_init,
            forward_ton_amount: settlement.transfer.forward_ton_amount,
            relay_amount: Some(required_outer),
        },
    })
}

/// Confirms the finalized trace contains the payer + source-jetton transfers.
///
/// # Errors
///
/// Returns a human-readable error if the expected txs are missing.
pub fn verify_finalized_trace_settlement(
    trace: &Value,
    settlement: &ParsedTvmSettlement,
    return_transaction: bool,
) -> Result<Value, String> {
    let transactions = parse_trace_transactions(trace).map_err(|e| e.to_string())?;
    let mut payer_transaction = None;
    for transaction in &transactions {
        if normalize_address_or_null(transaction.get("account")).as_ref() != Some(&settlement.payer)
        {
            continue;
        }
        if !transaction_succeeded(transaction) {
            continue;
        }
        if !message_body_hash_matches(transaction.get("in_msg"), &settlement.body) {
            continue;
        }
        payer_transaction = Some((*transaction).clone());
        break;
    }
    let payer_transaction = payer_transaction
        .ok_or_else(|| "Trace does not contain the expected payer wallet transaction".to_owned())?;

    let out_msgs = payer_transaction
        .get("out_msgs")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let mut payer_out_hash = None;
    let expected_b64 =
        crate::codecs::common::cell_hash_base64_from_ton_hash(&settlement.transfer.body_hash);
    for message in &out_msgs {
        if normalize_address_or_null(message.get("destination")).as_ref()
            != Some(&settlement.transfer.source_wallet)
        {
            continue;
        }
        let hash = message
            .get("message_content")
            .and_then(|c| c.get("hash"))
            .and_then(Value::as_str);
        if hash != Some(expected_b64.as_str()) {
            continue;
        }
        payer_out_hash = message.get("hash").cloned();
        break;
    }
    let payer_out_hash = payer_out_hash
        .ok_or_else(|| "Trace payer wallet transaction is missing out message hash".to_owned())?;

    let mut found_source = false;
    for transaction in &transactions {
        if normalize_address_or_null(transaction.get("account")).as_ref()
            != Some(&settlement.transfer.source_wallet)
        {
            continue;
        }
        if !transaction_succeeded(transaction) {
            continue;
        }
        if transaction.get("in_msg").and_then(|m| m.get("hash")) == Some(&payer_out_hash) {
            found_source = true;
            break;
        }
    }
    if !found_source {
        return Err(
            "Trace does not contain the expected source jetton wallet transaction".to_owned(),
        );
    }

    if return_transaction {
        return Ok(payer_transaction);
    }
    let hash = payer_transaction
        .get("hash_norm")
        .or_else(|| payer_transaction.get("hash"))
        .and_then(Value::as_str)
        .ok_or_else(|| "Trace payer wallet transaction is missing transaction hash".to_owned())?;
    Ok(Value::String(
        trace_transaction_hash_to_hex(hash).map_err(|e| e.to_string())?,
    ))
}

/// Hex transaction hash from a finalized trace (settle path).
///
/// # Errors
///
/// Returns a human-readable error if the expected txs are missing.
pub fn finalized_trace_tx_hash(
    trace: &Value,
    settlement: &ParsedTvmSettlement,
) -> Result<String, String> {
    match verify_finalized_trace_settlement(trace, settlement, false)? {
        Value::String(s) => Ok(s),
        other => Err(format!("unexpected trace hash value: {other}")),
    }
}

fn extra_from_json(value: Option<&Value>) -> Result<TvmExtra, TvmInvalid> {
    match value {
        None | Some(Value::Null) => Ok(TvmExtra::sponsored()),
        Some(v) => {
            serde_json::from_value(v.clone()).map_err(|_| invalid(ERR_EXACT_TVM_UNSUPPORTED_SCHEME))
        }
    }
}

fn normalize_json_address(value: Option<&Value>) -> Option<TvmAddress> {
    value.and_then(Value::as_str).and_then(|s| s.parse().ok())
}

fn json_u128(value: Option<&Value>) -> Option<u128> {
    match value? {
        Value::Number(n) => n.as_u64().map(u128::from),
        Value::String(s) => s.parse().ok(),
        _ => None,
    }
}

fn verify_ed25519(public_key: &[u8; 32], message: &[u8], signature: &[u8; 64]) -> bool {
    use ed25519_dalek::{Signature, Verifier, VerifyingKey};

    let Ok(vk) = VerifyingKey::from_bytes(public_key) else {
        return false;
    };
    let Ok(sig) = Signature::from_slice(signature) else {
        return false;
    };
    vk.verify(message, &sig).is_ok()
}

// Silence unused import if StateInitCells is only used via settlement.
#[allow(dead_code, reason = "re-exported type used by callers")]
fn _state_init_marker(_: &StateInitCells) {}

#[allow(dead_code, reason = "hash constant used by code-hash checks")]
const _: &str = W5R1_CODE_HASH;
