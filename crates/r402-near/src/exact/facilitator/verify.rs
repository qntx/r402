//! Facilitator verification for the NEAR exact scheme.

use base64::Engine;
use compact_str::CompactString;
use near_primitives::action::Action;
use near_primitives::action::delegate::SignedDelegateAction;
use near_primitives::gas::Gas;
use r402_protocol::error::ErrorReason;
use r402_protocol::payment::VerifyResponse;
use serde_json::Value;

use crate::chain::rpc::{
    NearAccessKeyPermissionKind, NearRpc, NearStorageBalance, parse_ft_transfer_args,
};
use crate::chain::{
    DEFAULT_MAX_SPONSORED_GAS, FT_TRANSFER_METHOD, NONCE_RANGE_MULTIPLIER, ONE_YOCTO,
    is_near_network, timeout_blocks,
};
use crate::exact::error::{NearInvalid, invalid};

/// Verifies a raw facilitator verify/settle request JSON.
///
/// Envelope checks run on the JSON so `invalid_x402_version` /
/// `unsupported_scheme` / `invalid_network` are emitted before typed decode.
pub async fn verify_request_json<R: NearRpc>(
    rpc: &R,
    relayer_ids: &[String],
    max_sponsored_gas: u64,
    request: &Value,
) -> VerifyResponse {
    match verify_inner(rpc, relayer_ids, max_sponsored_gas, request).await {
        Ok(payer) => VerifyResponse::valid(payer),
        Err(invalid) => invalid.into_response(),
    }
}

#[allow(
    clippy::too_many_lines,
    clippy::cognitive_complexity,
    reason = "sequential TS verify checklist must stay in one function"
)]
async fn verify_inner<R: NearRpc>(
    rpc: &R,
    relayer_ids: &[String],
    max_sponsored_gas: u64,
    request: &Value,
) -> Result<CompactString, NearInvalid> {
    let payload = request
        .get("paymentPayload")
        .ok_or_else(|| invalid("invalid_payload"))?;
    let requirements = request
        .get("paymentRequirements")
        .ok_or_else(|| invalid("invalid_payment_requirements"))?;

    // Version, scheme, network must fail closed before typed decode.
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
    if !is_near_network(req_network) {
        return Err(invalid("invalid_network"));
    }
    let accepted_network = accepted
        .get("network")
        .and_then(Value::as_str)
        .unwrap_or("");
    if accepted_network != req_network {
        return Err(invalid("invalid_exact_near_network_mismatch"));
    }

    if accepted.get("asset") != requirements.get("asset") {
        return Err(invalid("invalid_exact_near_asset_mismatch"));
    }
    if accepted.get("payTo") != requirements.get("payTo") {
        return Err(invalid("invalid_exact_near_pay_to_mismatch"));
    }
    if accepted.get("amount") != requirements.get("amount") {
        return Err(invalid("invalid_exact_near_amount_mismatch"));
    }
    if !is_positive_int(requirements.get("maxTimeoutSeconds")) {
        return Err(invalid("invalid_exact_near_max_timeout"));
    }
    let max_timeout_seconds = requirements
        .get("maxTimeoutSeconds")
        .and_then(Value::as_u64)
        .unwrap_or(0);

    let signed_b64 = payload
        .get("payload")
        .and_then(|p| p.get("signedDelegateAction"))
        .and_then(Value::as_str);
    let Some(signed_b64) = signed_b64 else {
        return Err(invalid("invalid_exact_near_payload_shape"));
    };

    let decoded = decode_signed_delegate(signed_b64)
        .map_err(|_| invalid("invalid_exact_near_payload_signed_delegate_action"))?;
    if !decoded.verify() {
        return Err(invalid("invalid_exact_near_payload_signature"));
    }
    let delegate = &decoded.delegate_action;
    let payer = CompactString::from(delegate.sender_id.as_str());

    if relayer_ids.is_empty() {
        return Err(invalid("invalid_exact_near_no_relayer_configured").with_payer(payer));
    }
    if relayer_ids.iter().any(|id| id == payer.as_str()) {
        return Err(invalid("invalid_exact_near_relayer_cannot_be_payer").with_payer(payer));
    }

    let actions = delegate.get_actions();
    if actions.len() != 1 {
        return Err(invalid("invalid_exact_near_payload_action_count").with_payer(payer));
    }
    let Some(Action::FunctionCall(fc)) = actions.into_iter().next() else {
        return Err(invalid("invalid_exact_near_payload_action_kind").with_payer(payer));
    };
    if fc.method_name != FT_TRANSFER_METHOD {
        return Err(invalid("invalid_exact_near_payload_method_name").with_payer(payer));
    }

    let asset = requirements
        .get("asset")
        .and_then(Value::as_str)
        .unwrap_or("");
    let pay_to = requirements
        .get("payTo")
        .and_then(Value::as_str)
        .unwrap_or("");
    let amount = requirements
        .get("amount")
        .and_then(Value::as_str)
        .unwrap_or("");
    if delegate.receiver_id.as_str() != asset {
        return Err(
            invalid("invalid_exact_near_payload_token_contract_mismatch").with_payer(payer),
        );
    }
    let transfer = parse_ft_transfer_args(&fc.args).map_err(|_| {
        invalid("invalid_exact_near_payload_ft_transfer_args").with_payer(payer.as_str())
    })?;
    if transfer.receiver_id != pay_to {
        return Err(invalid("invalid_exact_near_payload_recipient_mismatch").with_payer(payer));
    }
    if transfer.amount != amount {
        return Err(invalid("invalid_exact_near_payload_amount_mismatch").with_payer(payer));
    }
    if fc.deposit.as_yoctonear() != ONE_YOCTO {
        return Err(invalid("invalid_exact_near_payload_attached_deposit").with_payer(payer));
    }
    if fc.gas > Gas::from_gas(max_sponsored_gas) {
        return Err(invalid("invalid_exact_near_payload_gas_limit_exceeded").with_payer(payer));
    }

    let Ok(current_height) = rpc.current_block_height().await else {
        return Err(
            invalid("invalid_exact_near_current_block_height_unavailable").with_payer(payer),
        );
    };
    let timeout = timeout_blocks(max_timeout_seconds);
    if delegate.max_block_height <= current_height {
        return Err(
            invalid("invalid_exact_near_payload_delegate_action_expired").with_payer(payer),
        );
    }
    let remaining = delegate.max_block_height - current_height;
    if remaining > timeout {
        return Err(invalid(
            "invalid_exact_near_payload_delegate_action_timeout_window_exceeds_max_timeout",
        )
        .with_payer(payer));
    }
    let nonce_cap = current_height.saturating_mul(NONCE_RANGE_MULTIPLIER);
    if delegate.nonce >= nonce_cap {
        return Err(
            invalid("invalid_exact_near_payload_delegate_action_nonce_out_of_range")
                .with_payer(payer),
        );
    }

    let public_key = delegate.public_key.to_string();
    let Ok(access_key) = rpc
        .view_access_key(delegate.sender_id.as_str(), &public_key)
        .await
    else {
        return Err(invalid("invalid_exact_near_access_key_lookup_failed").with_payer(payer));
    };
    let Some(access_key) = access_key else {
        return Err(invalid("invalid_exact_near_access_key_not_found").with_payer(payer));
    };
    if delegate.nonce <= access_key.nonce {
        return Err(
            invalid("invalid_exact_near_payload_delegate_action_nonce_already_used")
                .with_payer(payer),
        );
    }

    match access_key.permission_kind {
        NearAccessKeyPermissionKind::FunctionCall => {
            return Err(
                invalid("invalid_exact_near_function_call_key_not_allowed").with_payer(payer)
            );
        }
        NearAccessKeyPermissionKind::Unknown => {
            return Err(
                invalid("invalid_exact_near_unsupported_access_key_permission").with_payer(payer),
            );
        }
        NearAccessKeyPermissionKind::FullAccess => {}
    }

    let Ok(sender_account) = rpc.view_account(delegate.sender_id.as_str()).await else {
        return Err(invalid("invalid_exact_near_account_lookup_failed").with_payer(payer));
    };
    if sender_account.is_none() {
        return Err(invalid("invalid_exact_near_sender_account_not_found").with_payer(payer));
    }

    let Ok(token_account) = rpc.view_account(asset).await else {
        return Err(invalid("invalid_exact_near_token_account_lookup_failed").with_payer(payer));
    };
    let Some(token_account) = token_account else {
        return Err(invalid("invalid_exact_near_token_account_not_found").with_payer(payer));
    };
    if token_account.has_no_code() {
        return Err(invalid("invalid_exact_near_token_contract_no_code").with_payer(payer));
    }

    let Ok(balance) = rpc.ft_balance_of(asset, delegate.sender_id.as_str()).await else {
        return Err(invalid("invalid_exact_near_balance_check_failed").with_payer(payer));
    };
    let required_amount: u128 = amount.parse().unwrap_or(u128::MAX);
    if balance < required_amount {
        return Err(NearInvalid {
            reason: ErrorReason::InsufficientFunds,
            payer: Some(payer),
        });
    }

    let Ok(storage) = rpc.storage_balance_of(asset, pay_to).await else {
        return Err(invalid("invalid_exact_near_storage_check_failed").with_payer(payer));
    };
    if matches!(storage, NearStorageBalance::Unregistered) {
        return Err(
            invalid("invalid_exact_near_recipient_not_registered_for_storage").with_payer(payer),
        );
    }

    Ok(payer)
}

/// Decodes a base64 Borsh `SignedDelegate`.
///
/// # Errors
///
/// Returns an error string when base64 or Borsh decoding fails.
pub fn decode_signed_delegate(b64: &str) -> Result<SignedDelegateAction, String> {
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(b64)
        .map_err(|e| e.to_string())?;
    borsh::BorshDeserialize::try_from_slice(&bytes).map_err(|e| e.to_string())
}

fn is_positive_int(value: Option<&Value>) -> bool {
    value.and_then(Value::as_u64).is_some_and(|n| n > 0)
}

/// Default sponsored-gas cap used when the caller does not override it.
#[must_use]
pub const fn default_max_sponsored_gas() -> u64 {
    DEFAULT_MAX_SPONSORED_GAS
}
