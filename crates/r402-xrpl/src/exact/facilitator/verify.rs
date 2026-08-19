//! Facilitator verification for the XRPL exact scheme.

use compact_str::CompactString;
use r402_core::wire::VerifyResponse;
use serde_json::Value;
use xrpl::core::keypairs::derive_classic_address;

use crate::chain::codec::{
    compare_decimal_strings, decode_signed_tx_blob, invoice_id_to_field,
    is_canonical_signing_pub_key, max_last_ledger_sequence, verify_tx_signature,
};
use crate::chain::rpc::{XrplRpc, unsigned_tx_for_simulate};
use crate::chain::types::{is_integer_string, is_xrpl_network, parse_xrpl_network_id};
use crate::exact::error::{XrplInvalid, invalid, invalid_owned};
use crate::exact::types::XrplAssetTransferMethod;
use crate::{DEFAULT_MAX_FEE_DROPS, TF_PARTIAL_PAYMENT};

/// Verifies a raw facilitator verify/settle request JSON.
pub async fn verify_request_json<R: XrplRpc>(
    rpc: &R,
    max_fee_drops: u64,
    request: &Value,
) -> VerifyResponse {
    match verify_inner(rpc, max_fee_drops, request).await {
        Ok(payer) => VerifyResponse::valid(payer),
        Err(invalid) => invalid.into_response(),
    }
}

#[allow(
    clippy::too_many_lines,
    clippy::cognitive_complexity,
    reason = "sequential verify checklist stays in one function"
)]
async fn verify_inner<R: XrplRpc>(
    rpc: &R,
    max_fee_drops: u64,
    request: &Value,
) -> Result<CompactString, XrplInvalid> {
    let payload = request
        .get("paymentPayload")
        .ok_or_else(|| invalid("invalid_payload"))?;
    let requirements = request
        .get("paymentRequirements")
        .ok_or_else(|| invalid("invalid_payment_requirements"))?;

    envelope_checks(payload, requirements)?;

    let method = resolve_asset_transfer_method(payload, requirements)?;

    let signed_blob = payload
        .get("payload")
        .and_then(|p| p.get("signedTxBlob"))
        .and_then(Value::as_str)
        .ok_or_else(|| invalid("invalid_exact_xrpl_payload_shape"))?;

    let decoded = decode_signed_tx_blob(signed_blob)
        .map_err(|_| invalid("invalid_exact_xrpl_payload_shape"))?;

    let signing_pub_key = decoded.get("SigningPubKey").and_then(Value::as_str);
    if signing_pub_key.is_none_or(|key| !is_canonical_signing_pub_key(key)) {
        return Err(invalid("invalid_exact_xrpl_payload_signing_pub_key"));
    }
    if !verify_tx_signature(signed_blob, &decoded) {
        return Err(invalid("invalid_exact_xrpl_payload_signature"));
    }

    if decoded.get("TransactionType").and_then(Value::as_str) != Some("Payment") {
        return Err(invalid("invalid_exact_xrpl_payload_transaction_type"));
    }

    let account = decoded
        .get("Account")
        .and_then(Value::as_str)
        .ok_or_else(|| invalid("invalid_exact_xrpl_payload_signature"))?;
    let payer = CompactString::from(account);

    verify_payment_structure(&decoded, requirements, max_fee_drops)
        .map_err(|e| e.with_payer(payer.as_str()))?;
    verify_sequencing_fields(&decoded, method).map_err(|e| e.with_payer(payer.as_str()))?;

    let req_network = requirements
        .get("network")
        .and_then(Value::as_str)
        .unwrap_or("");
    let current = rpc
        .current_ledger_index()
        .await
        .map_err(|_| invalid("invalid_exact_xrpl_facilitator_error").with_payer(payer.as_str()))?;
    verify_ledger_expiry(&decoded, requirements, current)
        .map_err(|e| e.with_payer(payer.as_str()))?;

    verify_signer_authorization(rpc, &decoded, req_network)
        .await
        .map_err(|e| e.with_payer(payer.as_str()))?;
    verify_sequencing_state(rpc, &decoded, method, req_network)
        .await
        .map_err(|e| e.with_payer(payer.as_str()))?;

    verify_simulation(rpc, &decoded)
        .await
        .map_err(|e| e.with_payer(payer.as_str()))?;

    Ok(payer)
}

fn envelope_checks(payload: &Value, requirements: &Value) -> Result<(), XrplInvalid> {
    if payload.get("x402Version").and_then(Value::as_u64) != Some(2) {
        return Err(invalid("invalid_x402_version"));
    }
    let accepted = payload
        .get("accepted")
        .ok_or_else(|| invalid("invalid_payload"))?;
    if accepted.get("scheme").and_then(Value::as_str) != Some("exact")
        || requirements.get("scheme").and_then(Value::as_str) != Some("exact")
    {
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
    if !is_xrpl_network(req_network) || !is_xrpl_network(accepted_network) {
        return Err(invalid("invalid_network"));
    }
    if accepted_network != req_network {
        return Err(invalid("invalid_exact_xrpl_network_mismatch"));
    }
    if accepted.get("asset") != requirements.get("asset") {
        return Err(invalid("invalid_exact_xrpl_asset_mismatch"));
    }
    if accepted.get("amount") != requirements.get("amount") {
        return Err(invalid("invalid_exact_xrpl_amount_mismatch"));
    }
    if accepted.get("payTo") != requirements.get("payTo") {
        return Err(invalid("invalid_exact_xrpl_pay_to_mismatch"));
    }
    if accepted.get("maxTimeoutSeconds") != requirements.get("maxTimeoutSeconds") {
        return Err(invalid("invalid_exact_xrpl_max_timeout_mismatch"));
    }
    let req_extra = requirements.get("extra");
    let acc_extra = accepted.get("extra");
    if json_bool(req_extra, "areFeesSponsored") != Some(false)
        || json_bool(acc_extra, "areFeesSponsored") != Some(false)
    {
        return Err(invalid("invalid_exact_xrpl_fees_sponsored_unsupported"));
    }
    if json_str(req_extra, "invoiceId") != json_str(acc_extra, "invoiceId") {
        return Err(invalid("invalid_exact_xrpl_invoice_mismatch"));
    }
    if json_u64(req_extra, "destinationTag") != json_u64(acc_extra, "destinationTag") {
        return Err(invalid("invalid_exact_xrpl_destination_tag_mismatch"));
    }
    let asset = requirements
        .get("asset")
        .and_then(Value::as_str)
        .unwrap_or("");
    if asset != "XRP" {
        let req_issuer = json_str(req_extra, "issuer");
        let acc_issuer = json_str(acc_extra, "issuer");
        if req_issuer.is_none() || acc_issuer.is_none() {
            return Err(invalid("invalid_exact_xrpl_iou_issuer_missing"));
        }
        if req_issuer != acc_issuer {
            return Err(invalid("invalid_exact_xrpl_iou_issuer_mismatch"));
        }
        let issuer = req_issuer.unwrap_or("");
        if issuer.parse::<crate::chain::XrplClassicAddress>().is_err() {
            return Err(invalid("invalid_exact_xrpl_iou_issuer_missing"));
        }
    }
    Ok(())
}

fn resolve_asset_transfer_method(
    payload: &Value,
    requirements: &Value,
) -> Result<XrplAssetTransferMethod, XrplInvalid> {
    let required = parse_method(json_str(requirements.get("extra"), "assetTransferMethod"))?;
    let accepted = parse_method(json_str(
        payload.get("accepted").and_then(|a| a.get("extra")),
        "assetTransferMethod",
    ))?;
    let selected = accepted
        .or(required)
        .unwrap_or(XrplAssetTransferMethod::Sequence);
    if let Some(required) = required
        && selected != required
    {
        return Err(invalid("invalid_exact_xrpl_asset_transfer_method_mismatch"));
    }
    Ok(selected)
}

fn parse_method(value: Option<&str>) -> Result<Option<XrplAssetTransferMethod>, XrplInvalid> {
    match value {
        None => Ok(None),
        Some("sequence") => Ok(Some(XrplAssetTransferMethod::Sequence)),
        Some("ticketSequence") => Ok(Some(XrplAssetTransferMethod::TicketSequence)),
        Some(_) => Err(invalid("invalid_exact_xrpl_asset_transfer_method")),
    }
}

fn verify_payment_structure(
    tx: &Value,
    requirements: &Value,
    max_fee_drops: u64,
) -> Result<(), XrplInvalid> {
    let pay_to = requirements
        .get("payTo")
        .and_then(Value::as_str)
        .unwrap_or("");
    if tx.get("Destination").and_then(Value::as_str) != Some(pay_to) {
        return Err(invalid("invalid_exact_xrpl_payload_destination_mismatch"));
    }
    if let Some(tag) = json_u64(requirements.get("extra"), "destinationTag") {
        if tag > u64::from(u32::MAX) {
            return Err(invalid("invalid_exact_xrpl_destination_tag_malformed"));
        }
        if json_u64(Some(tx), "DestinationTag") != Some(tag) {
            return Err(invalid(
                "invalid_exact_xrpl_payload_destination_tag_mismatch",
            ));
        }
    }
    if tx.get("Delegate").is_some() {
        return Err(invalid("invalid_exact_xrpl_payload_delegate_not_allowed"));
    }
    if tx.get("Signers").is_some() {
        return Err(invalid("invalid_exact_xrpl_payload_multisig_not_supported"));
    }
    verify_network_binding(
        tx,
        requirements
            .get("network")
            .and_then(Value::as_str)
            .unwrap_or(""),
    )?;
    let asset = requirements
        .get("asset")
        .and_then(Value::as_str)
        .unwrap_or("");
    if asset == "XRP" {
        verify_xrp_amount(tx, requirements)?;
    } else {
        verify_iou_amount(tx, requirements)?;
    }
    verify_invoice_binding(tx, requirements)?;
    verify_fee(tx, max_fee_drops)?;
    Ok(())
}

fn verify_network_binding(tx: &Value, network: &str) -> Result<(), XrplInvalid> {
    let network_id = parse_xrpl_network_id(network).map_err(|_| invalid("invalid_network"))?;
    let tx_network = json_u64(Some(tx), "NetworkID");
    if network_id <= 1024 && tx_network.is_some() {
        return Err(invalid(
            "invalid_exact_xrpl_payload_network_id_for_standard_network",
        ));
    }
    if network_id > 1024 && tx_network != Some(u64::from(network_id)) {
        return Err(invalid("invalid_exact_xrpl_payload_network_id_mismatch"));
    }
    Ok(())
}

fn destination_amount(tx: &Value) -> Result<&Value, XrplInvalid> {
    match (tx.get("Amount"), tx.get("DeliverMax")) {
        (Some(_), Some(_)) => Err(invalid("invalid_exact_xrpl_facilitator_error")),
        (None, None) => Err(invalid("invalid_exact_xrpl_payload_amount_xrp")),
        (Some(amount), None) | (None, Some(amount)) => Ok(amount),
    }
}

fn verify_xrp_amount(tx: &Value, requirements: &Value) -> Result<(), XrplInvalid> {
    let dest = destination_amount(tx)?;
    let dest_str = dest.as_str().unwrap_or("");
    if !is_integer_string(dest_str) {
        return Err(invalid("invalid_exact_xrpl_payload_amount_xrp"));
    }
    let expected = requirements
        .get("amount")
        .and_then(Value::as_str)
        .unwrap_or("");
    if dest_str.parse::<u128>().ok() != expected.parse::<u128>().ok() {
        return Err(invalid("invalid_exact_xrpl_payload_amount_mismatch"));
    }
    reject_path_fields(tx)?;
    if tx.get("SendMax").is_some() {
        return Err(invalid("invalid_exact_xrpl_payload_sendmax_not_allowed"));
    }
    Ok(())
}

fn verify_iou_amount(tx: &Value, requirements: &Value) -> Result<(), XrplInvalid> {
    let dest = destination_amount(tx)?;
    let dest_obj = dest
        .as_object()
        .ok_or_else(|| invalid("invalid_exact_xrpl_payload_iou_amount"))?;
    let currency = dest_obj
        .get("currency")
        .and_then(Value::as_str)
        .unwrap_or("");
    let issuer = dest_obj.get("issuer").and_then(Value::as_str).unwrap_or("");
    let value = dest_obj.get("value").and_then(Value::as_str).unwrap_or("");
    let asset = requirements
        .get("asset")
        .and_then(Value::as_str)
        .unwrap_or("");
    if currency != asset {
        return Err(invalid("invalid_exact_xrpl_payload_iou_currency_mismatch"));
    }
    let expected_issuer = json_str(requirements.get("extra"), "issuer").unwrap_or("");
    if issuer != expected_issuer {
        return Err(invalid("invalid_exact_xrpl_payload_iou_issuer_mismatch"));
    }
    let expected_value = requirements
        .get("amount")
        .and_then(Value::as_str)
        .unwrap_or("");
    match compare_decimal_strings(value, expected_value) {
        Ok(std::cmp::Ordering::Equal) => {}
        Ok(_) => return Err(invalid("invalid_exact_xrpl_payload_iou_value_mismatch")),
        Err(_) => return Err(invalid("invalid_exact_xrpl_payload_iou_amount")),
    }
    let send_max = tx
        .get("SendMax")
        .and_then(Value::as_object)
        .ok_or_else(|| invalid("invalid_exact_xrpl_payload_sendmax_required"))?;
    if send_max.get("currency").and_then(Value::as_str) != Some(currency)
        || send_max.get("issuer").and_then(Value::as_str) != Some(issuer)
    {
        return Err(invalid("invalid_exact_xrpl_payload_sendmax_iou_mismatch"));
    }
    let send_value = send_max.get("value").and_then(Value::as_str).unwrap_or("");
    match compare_decimal_strings(send_value, value) {
        Ok(std::cmp::Ordering::Less) => {
            return Err(invalid("invalid_exact_xrpl_payload_sendmax_too_low"));
        }
        Ok(_) => {}
        Err(_) => return Err(invalid("invalid_exact_xrpl_payload_sendmax_required")),
    }
    reject_path_fields(tx)?;
    Ok(())
}

fn reject_path_fields(tx: &Value) -> Result<(), XrplInvalid> {
    if tx.get("Paths").is_some() {
        return Err(invalid("invalid_exact_xrpl_payload_paths_not_allowed"));
    }
    if tx.get("DeliverMin").is_some() {
        return Err(invalid("invalid_exact_xrpl_payload_delivermin_not_allowed"));
    }
    if has_partial_payment_flag(tx.get("Flags")) {
        return Err(invalid(
            "invalid_exact_xrpl_payload_partial_payment_not_allowed",
        ));
    }
    Ok(())
}

fn has_partial_payment_flag(flags: Option<&Value>) -> bool {
    match flags {
        Some(Value::Number(n)) => n
            .as_u64()
            .is_some_and(|v| (v & u64::from(TF_PARTIAL_PAYMENT)) != 0),
        Some(Value::Object(obj)) => obj.get("tfPartialPayment") == Some(&Value::Bool(true)),
        _ => false,
    }
}

fn verify_invoice_binding(tx: &Value, requirements: &Value) -> Result<(), XrplInvalid> {
    if tx.get("Memos").is_some() {
        return Err(invalid("invalid_exact_xrpl_payload_memos_not_allowed"));
    }
    let Some(invoice_id) = json_str(requirements.get("extra"), "invoiceId") else {
        return Ok(());
    };
    if invoice_id.is_empty() {
        return Err(invalid("invalid_exact_xrpl_payload_invoice_missing"));
    }
    let expected = invoice_id_to_field(invoice_id);
    let Some(actual) = tx.get("InvoiceID").and_then(Value::as_str) else {
        return Err(invalid("invalid_exact_xrpl_payload_invoice_missing"));
    };
    if !actual.eq_ignore_ascii_case(&expected) {
        return Err(invalid("invalid_exact_xrpl_payload_invoice_id_mismatch"));
    }
    Ok(())
}

fn verify_fee(tx: &Value, max_fee_drops: u64) -> Result<(), XrplInvalid> {
    let fee = tx.get("Fee").and_then(Value::as_str).unwrap_or("");
    if !is_integer_string(fee) {
        return Err(invalid("invalid_exact_xrpl_payload_fee_missing"));
    }
    let fee_drops: u64 = fee
        .parse()
        .map_err(|_| invalid("invalid_exact_xrpl_payload_fee_missing"))?;
    if fee_drops > max_fee_drops {
        return Err(invalid("invalid_exact_xrpl_payload_fee_too_high"));
    }
    Ok(())
}

fn verify_ledger_expiry(
    tx: &Value,
    requirements: &Value,
    current_ledger: u32,
) -> Result<(), XrplInvalid> {
    let last = json_u64(Some(tx), "LastLedgerSequence")
        .and_then(|v| u32::try_from(v).ok())
        .ok_or_else(|| invalid("invalid_exact_xrpl_payload_lastledgersequence_missing"))?;
    if last <= current_ledger {
        return Err(invalid("invalid_exact_xrpl_payload_expired"));
    }
    let timeout = requirements
        .get("maxTimeoutSeconds")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    if last > max_last_ledger_sequence(current_ledger, timeout) {
        return Err(invalid(
            "invalid_exact_xrpl_payload_lastledgersequence_too_large",
        ));
    }
    Ok(())
}

fn verify_sequencing_fields(
    tx: &Value,
    method: XrplAssetTransferMethod,
) -> Result<(), XrplInvalid> {
    match method {
        XrplAssetTransferMethod::Sequence => {
            if tx.get("TicketSequence").is_some() {
                return Err(invalid(
                    "invalid_exact_xrpl_payload_ticket_sequence_not_allowed",
                ));
            }
            let seq = json_u64(Some(tx), "Sequence").unwrap_or(0);
            if seq == 0 {
                return Err(invalid("invalid_exact_xrpl_payload_sequence_missing"));
            }
        }
        XrplAssetTransferMethod::TicketSequence => {
            if json_u64(Some(tx), "Sequence") != Some(0) {
                return Err(invalid("invalid_exact_xrpl_payload_sequence_must_be_zero"));
            }
            if json_u64(Some(tx), "TicketSequence").is_none() {
                return Err(invalid(
                    "invalid_exact_xrpl_payload_ticket_sequence_missing",
                ));
            }
        }
    }
    Ok(())
}

async fn verify_signer_authorization<R: XrplRpc>(
    rpc: &R,
    tx: &Value,
    _network: &str,
) -> Result<(), XrplInvalid> {
    let signing_pub_key = tx
        .get("SigningPubKey")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| invalid("invalid_exact_xrpl_payload_signature"))?;
    let signer_address = derive_classic_address(signing_pub_key)
        .map_err(|_| invalid("invalid_exact_xrpl_payload_signature"))?;
    let account = tx.get("Account").and_then(Value::as_str).unwrap_or("");
    let auth = rpc
        .account_authorization(account)
        .await
        .map_err(|_| invalid("invalid_exact_xrpl_facilitator_error"))?;
    if auth.regular_key.as_deref() == Some(signer_address.as_str()) {
        return Ok(());
    }
    if signer_address == account && !auth.is_master_key_disabled {
        return Ok(());
    }
    Err(invalid("invalid_exact_xrpl_payload_signer_not_authorized"))
}

async fn verify_sequencing_state<R: XrplRpc>(
    rpc: &R,
    tx: &Value,
    method: XrplAssetTransferMethod,
    _network: &str,
) -> Result<(), XrplInvalid> {
    let account = tx.get("Account").and_then(Value::as_str).unwrap_or("");
    match method {
        XrplAssetTransferMethod::Sequence => {
            let auth = rpc
                .account_authorization(account)
                .await
                .map_err(|_| invalid("invalid_exact_xrpl_facilitator_error"))?;
            let seq = json_u64(Some(tx), "Sequence").and_then(|v| u32::try_from(v).ok());
            if seq != Some(auth.sequence) {
                return Err(invalid("invalid_exact_xrpl_payload_sequence_not_current"));
            }
        }
        XrplAssetTransferMethod::TicketSequence => {
            let ticket = json_u64(Some(tx), "TicketSequence")
                .and_then(|v| u32::try_from(v).ok())
                .ok_or_else(|| invalid("invalid_exact_xrpl_payload_ticket_sequence_missing"))?;
            let tickets = rpc
                .ticket_sequences(account)
                .await
                .map_err(|_| invalid("invalid_exact_xrpl_facilitator_error"))?;
            if !tickets.contains(&ticket) {
                return Err(invalid("invalid_exact_xrpl_payload_ticket_not_available"));
            }
        }
    }
    Ok(())
}

async fn verify_simulation<R: XrplRpc>(rpc: &R, decoded: &Value) -> Result<(), XrplInvalid> {
    let unsigned = unsigned_tx_for_simulate(decoded);
    let result = rpc
        .simulate(&unsigned)
        .await
        .map_err(|_| invalid("invalid_exact_xrpl_facilitator_error"))?;
    if result.engine_result != "tesSUCCESS" {
        return Err(invalid_owned(format!(
            "invalid_exact_xrpl_payload_simulation_failed: {}",
            result.engine_result
        )));
    }
    Ok(())
}

fn json_bool(extra: Option<&Value>, key: &str) -> Option<bool> {
    extra.and_then(|v| v.get(key)).and_then(Value::as_bool)
}

fn json_str<'a>(extra: Option<&'a Value>, key: &str) -> Option<&'a str> {
    extra.and_then(|v| v.get(key)).and_then(Value::as_str)
}

fn json_u64(extra: Option<&Value>, key: &str) -> Option<u64> {
    extra.and_then(|v| v.get(key)).and_then(Value::as_u64)
}

/// Default max fee used when the caller does not override it.
#[must_use]
pub const fn default_max_fee_drops() -> u64 {
    DEFAULT_MAX_FEE_DROPS
}
