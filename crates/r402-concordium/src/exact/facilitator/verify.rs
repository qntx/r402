//! Facilitator verification for the Concordium exact scheme.

use std::time::{SystemTime, UNIX_EPOCH};

use compact_str::CompactString;
use r402_protocol::payment::VerifyResponse;
use serde_json::Value;

use crate::chain::account::{ConcordiumAddress, is_native_ccd};
use crate::chain::rpc::ConcordiumNode;
use crate::chain::tx::{DecodedPayload, decode_payload, has_sender_signature, rebuild_for_verify};
use crate::exact::error::{ConcordiumInvalid, invalid};
use crate::exact::payload::{
    SignableV1Signatures, SignableV1Transaction, SignableV1TransactionHeader,
    SignableV1TransactionPayload, SponsorHeader,
};

/// Verifies a raw facilitator verify/settle request JSON.
pub async fn verify_request_json<N: ConcordiumNode>(
    node: &N,
    facilitator_addresses: &[String],
    request: &Value,
    max_expiry_offset_seconds: u64,
) -> VerifyResponse {
    match verify_inner(
        node,
        facilitator_addresses,
        request,
        max_expiry_offset_seconds,
    )
    .await
    {
        Ok(payer) => VerifyResponse::valid(payer),
        Err(invalid) => invalid.into_response(),
    }
}

#[allow(
    clippy::too_many_lines,
    clippy::cognitive_complexity,
    reason = "sequential Concordium verify checklist stays in one function"
)]
async fn verify_inner<N: ConcordiumNode>(
    node: &N,
    facilitator_addresses: &[String],
    request: &Value,
    max_expiry_offset_seconds: u64,
) -> Result<CompactString, ConcordiumInvalid> {
    let payload = request
        .get("paymentPayload")
        .ok_or_else(|| invalid("missing_payload"))?;
    let requirements = request
        .get("paymentRequirements")
        .ok_or_else(|| invalid("missing_payload"))?;

    let accepted = payload.get("accepted").unwrap_or(payload);
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
    let inner = payload.get("payload").unwrap_or(&Value::Null);
    if inner.is_null() || !inner.is_object() {
        return Err(invalid("missing_payload"));
    }

    let tx = parse_transaction(inner)
        .map_err(|reason| invalid(format!("invalid_transaction_format: {reason}")))?;
    let resolved_payer = tx.header.sender.clone();

    if tx.version != 1 {
        return Err(invalid(format!(
            "invalid_transaction_version: expected 1, got {}",
            tx.version
        ))
        .with_payer(resolved_payer));
    }
    if resolved_payer.is_empty() {
        return Err(invalid("missing_sender").with_payer(resolved_payer));
    }
    if ConcordiumAddress::decode(&resolved_payer).is_err() {
        return Err(invalid("invalid_sender_address").with_payer(resolved_payer));
    }

    let fee_payer = requirements
        .get("extra")
        .and_then(|e| e.get("feePayer"))
        .and_then(Value::as_str)
        .unwrap_or("");
    if fee_payer.is_empty() {
        return Err(invalid("missing_fee_payer").with_payer(resolved_payer));
    }
    if !facilitator_addresses.iter().any(|a| a == fee_payer) {
        return Err(invalid("fee_payer_not_managed_by_facilitator").with_payer(resolved_payer));
    }

    let sponsor_in_header = tx
        .header
        .sponsor
        .as_ref()
        .and_then(SponsorHeader::resolved_address)
        .unwrap_or("");
    if sponsor_in_header.is_empty() {
        return Err(invalid("missing_sponsor_in_header").with_payer(resolved_payer));
    }
    if sponsor_in_header != fee_payer {
        return Err(invalid("sponsor_mismatch").with_payer(resolved_payer));
    }

    let now = now_unix();
    let expiry = parse_expiry(&tx.header.expiry)
        .ok_or_else(|| invalid("invalid_expiry_field").with_payer(resolved_payer.clone()))?;
    if expiry <= now {
        return Err(invalid("transaction_expired").with_payer(resolved_payer));
    }
    let req_timeout = requirements
        .get("maxTimeoutSeconds")
        .and_then(Value::as_u64)
        .unwrap_or(max_expiry_offset_seconds);
    let max_offset = max_expiry_offset_seconds.min(req_timeout);
    if expiry > now.saturating_add(max_offset) {
        return Err(invalid(format!(
            "expiry_too_far_in_future: max offset is {max_offset}s"
        ))
        .with_payer(resolved_payer));
    }

    let decoded = match decode_payload(&tx.payload) {
        Ok(d) => d,
        Err(reason) => return Err(invalid(reason).with_payer(resolved_payer)),
    };
    if facilitator_addresses.iter().any(|a| a == &resolved_payer) {
        return Err(invalid("sponsor_as_sender").with_payer(resolved_payer));
    }

    let expected_asset = requirements
        .get("asset")
        .and_then(Value::as_str)
        .unwrap_or("CCD")
        .to_ascii_uppercase();
    if let Some(err) = check_asset_type(&tx.payload, &expected_asset) {
        return Err(invalid(err).with_payer(resolved_payer));
    }
    let pay_to = requirements
        .get("payTo")
        .and_then(Value::as_str)
        .unwrap_or("");
    if let Some(err) = check_recipient(&tx.payload, pay_to, &expected_asset, &decoded) {
        return Err(invalid(err).with_payer(resolved_payer));
    }
    let required_amount = requirements
        .get("amount")
        .and_then(Value::as_str)
        .unwrap_or("");
    if let Some(err) = check_amount(
        &tx.payload,
        required_amount,
        &expected_asset,
        &decoded,
        node,
    )
    .await
    {
        return Err(invalid(err).with_payer(resolved_payer));
    }
    if !has_sender_signature(&tx.signatures.sender) {
        return Err(invalid("missing_sender_signature").with_payer(resolved_payer));
    }

    let rebuilt = rebuild_for_verify(&tx).map_err(|e| {
        invalid(format!("signature_verification_failed: {e}")).with_payer(resolved_payer.clone())
    })?;
    let account = node.get_account(&resolved_payer).await.map_err(|e| {
        invalid(format!("signature_verification_failed: {e}")).with_payer(resolved_payer.clone())
    })?;
    let Some(info) = account.info.as_ref() else {
        return Err(
            invalid("signature_verification_failed: missing account credentials")
                .with_payer(resolved_payer),
        );
    };
    if !crate::chain::tx::verify_sender_signature(&rebuilt, info) {
        return Err(invalid("invalid_sender_signature").with_payer(resolved_payer));
    }
    if let Some(err) = preflight_likely_to_succeed(
        &tx,
        required_amount,
        &expected_asset,
        &account,
        node,
        &decoded,
    )
    .await
    {
        return Err(invalid(err).with_payer(resolved_payer));
    }
    Ok(CompactString::from(resolved_payer))
}

/// Spec preflight: nonce match + sufficient CCD/PLT balance.
pub(crate) async fn preflight_likely_to_succeed<N: ConcordiumNode>(
    tx: &SignableV1Transaction,
    required_amount: &str,
    expected_asset: &str,
    account: &crate::chain::rpc::AccountSnapshot,
    node: &N,
    decoded: &DecodedPayload,
) -> Option<String> {
    let Some(on_chain_nonce) = account.nonce else {
        return Some("preflight_missing_account_nonce".to_owned());
    };
    if tx.header.nonce != on_chain_nonce {
        return Some("preflight_nonce_mismatch".to_owned());
    }
    if !is_native_ccd(expected_asset) {
        let (Some(token_id), Some(amount)) = (decoded.token_id.as_deref(), decoded.amount) else {
            return Some("preflight_invalid_token_amount".to_owned());
        };
        return preflight_token_balance(node, &tx.header.sender, token_id, amount).await;
    }
    if !required_amount.bytes().all(|b| b.is_ascii_digit()) || required_amount.is_empty() {
        return Some("invalid_required_amount".to_owned());
    }
    let required: u128 = required_amount.parse().ok()?;
    let Some(available) = account.amount_micro_ccd else {
        return Some("preflight_missing_account_amount".to_owned());
    };
    if u128::from(available) < required {
        return Some("preflight_insufficient_funds".to_owned());
    }
    None
}

async fn preflight_token_balance<N: ConcordiumNode>(
    node: &N,
    payer: &str,
    token_id: &str,
    required: u128,
) -> Option<String> {
    match node.get_token_balance(payer, token_id).await {
        Err(_) => Some("preflight_token_balance_lookup_failed".to_owned()),
        Ok(None) => Some("preflight_missing_token_balance".to_owned()),
        Ok(Some(balance)) if balance < required => {
            Some("preflight_insufficient_token_funds".to_owned())
        }
        Ok(Some(_)) => None,
    }
}

fn check_asset_type(payload: &SignableV1TransactionPayload, expected: &str) -> Option<String> {
    if is_native_ccd(expected) {
        return match payload {
            SignableV1TransactionPayload::Transfer { .. }
            | SignableV1TransactionPayload::TransferWithMemo { .. } => None,
            other @ SignableV1TransactionPayload::TokenUpdate { .. } => Some(format!(
                "asset_type_mismatch: expected SimpleTransfer for CCD, got {}",
                other.type_name()
            )),
        };
    }
    match payload {
        SignableV1TransactionPayload::TokenUpdate { token_id, .. } => {
            let Some(id) = token_id.as_ref().filter(|s| !s.is_empty()) else {
                return Some("missing_token_id".to_owned());
            };
            if !id.eq_ignore_ascii_case(expected) {
                return Some(format!("token_id_mismatch: expected {expected}, got {id}"));
            }
            None
        }
        other => Some(format!(
            "asset_type_mismatch: expected TokenUpdate for {expected}, got {}",
            other.type_name()
        )),
    }
}

fn check_recipient(
    payload: &SignableV1TransactionPayload,
    pay_to: &str,
    expected: &str,
    decoded: &DecodedPayload,
) -> Option<String> {
    if is_native_ccd(expected) {
        let to = match payload {
            SignableV1TransactionPayload::Transfer { to_address, .. }
            | SignableV1TransactionPayload::TransferWithMemo { to_address, .. } => to_address,
            SignableV1TransactionPayload::TokenUpdate { .. } => {
                return Some("missing_recipient".to_owned());
            }
        };
        let Some(to) = to.as_ref().filter(|s| !s.is_empty()) else {
            return Some("missing_recipient".to_owned());
        };
        if to != pay_to {
            return Some("recipient_mismatch".to_owned());
        }
        return None;
    }
    let Some(recipient) = decoded.recipient.as_deref() else {
        return Some("missing_recipient".to_owned());
    };
    if recipient != pay_to {
        return Some("recipient_mismatch".to_owned());
    }
    None
}

async fn check_amount<N: ConcordiumNode>(
    payload: &SignableV1TransactionPayload,
    required_amount: &str,
    expected: &str,
    decoded: &DecodedPayload,
    node: &N,
) -> Option<String> {
    if !required_amount.bytes().all(|b| b.is_ascii_digit()) || required_amount.is_empty() {
        return Some("invalid_required_amount".to_owned());
    }
    let required: u128 = match required_amount.parse() {
        Ok(v) => v,
        Err(_) => return Some("invalid_required_amount".to_owned()),
    };
    if !is_native_ccd(expected) {
        let Some(actual) = decoded.amount else {
            return Some("invalid_amount_format".to_owned());
        };
        let (Some(decimals), Some(token_id)) =
            (decoded.token_decimals, decoded.token_id.as_deref())
        else {
            return Some("invalid_token_amount".to_owned());
        };
        let Ok(expected_decimals) = node.get_token_decimals(token_id).await else {
            return Some("token_decimals_lookup_failed".to_owned());
        };
        if decimals != expected_decimals {
            return Some("invalid_token_amount_decimals".to_owned());
        }
        return (actual != required)
            .then(|| format!("amount_mismatch: required {required}, got {actual}"));
    }
    let amount_val = match payload {
        SignableV1TransactionPayload::Transfer { amount, .. }
        | SignableV1TransactionPayload::TransferWithMemo { amount, .. } => amount,
        SignableV1TransactionPayload::TokenUpdate { .. } => {
            return Some("invalid_amount_format".to_owned());
        }
    };
    let Some(actual) = json_u128(amount_val) else {
        return Some("invalid_amount_format".to_owned());
    };
    (actual != required).then(|| format!("amount_mismatch: required {required}, got {actual}"))
}

fn json_u128(value: &Value) -> Option<u128> {
    match value {
        Value::String(s) => s.parse().ok(),
        Value::Number(n) => n.as_u64().map(u128::from),
        _ => None,
    }
}

/// Parses the inner Concordium payload object.
pub(super) fn parse_transaction(inner: &Value) -> Result<SignableV1Transaction, String> {
    let signed = inner
        .get("signedTransaction")
        .ok_or_else(|| "missing_signed_transaction".to_owned())?;
    if !signed.is_object() {
        return Err("signed_transaction_must_be_object".to_owned());
    }
    let version = signed
        .get("version")
        .and_then(Value::as_u64)
        .ok_or_else(|| "missing_or_invalid_version_field".to_owned())?;
    let header_val = signed
        .get("header")
        .ok_or_else(|| "missing_header".to_owned())?;
    if !header_val.is_object() {
        return Err("missing_header".to_owned());
    }
    let sender = header_val
        .get("sender")
        .and_then(Value::as_str)
        .ok_or_else(|| "missing_header_sender".to_owned())?
        .to_owned();
    // Official parseTransaction: expiry MUST be a JSON number.
    if !header_val.get("expiry").is_some_and(Value::is_number) {
        return Err("missing_header_expiry".to_owned());
    }
    let expiry = header_val
        .get("expiry")
        .cloned()
        .ok_or_else(|| "missing_header_expiry".to_owned())?;
    let sponsor = header_val
        .get("sponsor")
        .filter(|v| v.is_object())
        .cloned()
        .ok_or_else(|| "missing_header_sponsor".to_owned())?;
    let payload_val = signed
        .get("payload")
        .ok_or_else(|| "missing_payload_field".to_owned())?;
    if !payload_val.is_object() {
        return Err("missing_payload_field".to_owned());
    }
    let signatures = signed
        .get("signatures")
        .filter(|v| v.is_object())
        .cloned()
        .ok_or_else(|| "missing_signatures".to_owned())?;
    let header = SignableV1TransactionHeader {
        sender,
        nonce: header_val.get("nonce").and_then(Value::as_u64).unwrap_or(0),
        expiry,
        num_signatures: header_val
            .get("numSignatures")
            .and_then(Value::as_u64)
            .unwrap_or(1),
        execution_energy_amount: header_val
            .get("executionEnergyAmount")
            .and_then(Value::as_u64),
        sponsor: serde_json::from_value::<SponsorHeader>(sponsor).ok(),
    };
    let payload: SignableV1TransactionPayload =
        serde_json::from_value(payload_val.clone()).or_else(|_| fallback_payload(payload_val))?;
    let signatures: SignableV1Signatures = serde_json::from_value(signatures).unwrap_or_default();
    Ok(SignableV1Transaction {
        version,
        header,
        payload,
        signatures,
    })
}

fn fallback_payload(value: &Value) -> Result<SignableV1TransactionPayload, String> {
    let ty = value
        .get("type")
        .or_else(|| value.get("transactionType"))
        .and_then(Value::as_str)
        .unwrap_or("");
    match ty {
        "transfer" => Ok(SignableV1TransactionPayload::Transfer {
            to_address: value
                .get("toAddress")
                .and_then(Value::as_str)
                .map(str::to_owned),
            amount: value.get("amount").cloned().unwrap_or(Value::Null),
        }),
        "transferWithMemo" => Ok(SignableV1TransactionPayload::TransferWithMemo {
            to_address: value
                .get("toAddress")
                .and_then(Value::as_str)
                .map(str::to_owned),
            amount: value.get("amount").cloned().unwrap_or(Value::Null),
            memo: value.get("memo").and_then(Value::as_str).map(str::to_owned),
        }),
        "tokenUpdate" => Ok(SignableV1TransactionPayload::TokenUpdate {
            token_id: value
                .get("tokenId")
                .and_then(Value::as_str)
                .map(str::to_owned),
            operations: value.get("operations").cloned().unwrap_or(Value::Null),
        }),
        _ => Err(format!("unexpected_transaction_type: {ty}")),
    }
}

fn parse_expiry(value: &Value) -> Option<u64> {
    match value {
        Value::Number(n) => n.as_u64(),
        Value::String(s) => s.parse().ok(),
        _ => None,
    }
}

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}
