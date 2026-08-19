//! Facilitator verification for the Keeta exact scheme.

use base64::Engine;
use compact_str::CompactString;
use keetanetwork_block::{Amount, Block, Operation};
use r402_core::wire::VerifyResponse;
use serde_json::Value;

use crate::chain::provider::KeetaPreflight;
use crate::chain::types::is_keeta_network;
use crate::exact::error::{KeetaInvalid, invalid};

/// Verifies a raw facilitator verify/settle request JSON.
///
/// Envelope checks run on the JSON so TS `invalidReason` codes are emitted
/// before typed decode.
pub async fn verify_request_json<P: KeetaPreflight>(
    preflight: &P,
    fee_payer_ids: &[String],
    request: &Value,
) -> VerifyResponse {
    match verify_inner(preflight, fee_payer_ids, request).await {
        Ok(payer) => VerifyResponse::valid(payer),
        Err(invalid) => invalid.into_response(),
    }
}

#[allow(
    clippy::too_many_lines,
    clippy::cognitive_complexity,
    reason = "sequential TS verify checklist must stay in one function"
)]
async fn verify_inner<P: KeetaPreflight>(
    preflight: &P,
    fee_payer_ids: &[String],
    request: &Value,
) -> Result<CompactString, KeetaInvalid> {
    let payload = request
        .get("paymentPayload")
        .ok_or_else(|| invalid("invalid_payload"))?;
    let requirements = request
        .get("paymentRequirements")
        .ok_or_else(|| invalid("invalid_payment_requirements"))?;

    if payload.get("x402Version").and_then(Value::as_u64) != Some(2) {
        return Err(invalid("invalid_exact_keeta_payload_unsupported_version"));
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
    let (namespace, network_id) = parse_caip_network(req_network)?;
    let accepted_network = accepted
        .get("network")
        .and_then(Value::as_str)
        .unwrap_or("");
    if accepted_network != req_network {
        return Err(invalid("network_mismatch"));
    }

    let block_b64 = payload
        .get("payload")
        .and_then(|p| p.get("block"))
        .and_then(Value::as_str)
        .ok_or_else(|| invalid("invalid_exact_keeta_payload_block_could_not_be_decoded"))?;
    let block = decode_block(block_b64)
        .map_err(|_| invalid("invalid_exact_keeta_payload_block_could_not_be_decoded"))?;

    if namespace != "keeta"
        || parse_network_id(&block.data().network().to_string()) != Some(network_id)
    {
        return Err(invalid("network_mismatch"));
    }
    if !is_keeta_network(req_network) {
        return Err(invalid("network_mismatch"));
    }

    let operations = block.data().operations();
    if operations.len() != 1 {
        return Err(invalid("invalid_exact_keeta_payload_operations_length"));
    }
    let Some(Operation::Send(send)) = operations.first() else {
        return Err(invalid(
            "invalid_exact_keeta_payload_payment_operation_type",
        ));
    };

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

    if send.token.to_string() != asset {
        return Err(invalid(
            "invalid_exact_keeta_payload_payment_asset_mismatch",
        ));
    }
    let required_amount: Amount = amount
        .parse()
        .map_err(|_| invalid("invalid_exact_keeta_payload_payment_amount_invalid"))?;
    if send.amount != required_amount {
        return Err(invalid(
            "invalid_exact_keeta_payload_payment_amount_mismatch",
        ));
    }
    if send.to.to_string() != pay_to {
        return Err(invalid("invalid_exact_keeta_payload_payment_to_mismatch"));
    }
    if let Some(required_external) = extra_external(requirements)
        && send.external.as_deref() != Some(required_external)
    {
        return Err(invalid(
            "invalid_exact_keeta_payload_payment_external_mismatch",
        ));
    }

    let payer = CompactString::from(block.data().account().to_string());
    if fee_payer_ids.iter().any(|id| id == payer.as_str()) {
        return Err(invalid("invalid_exact_keeta_payload_payer_is_facilitator").with_payer(payer));
    }

    simulate(
        preflight,
        &block,
        send.token.to_string().as_str(),
        &required_amount,
        payer.as_str(),
    )
    .await
    .map_err(|err| err.with_payer(payer.as_str()))?;
    Ok(payer)
}

async fn simulate<P: KeetaPreflight>(
    preflight: &P,
    block: &Block,
    token: &str,
    required_amount: &Amount,
    payer: &str,
) -> Result<(), KeetaInvalid> {
    let balance = preflight
        .balance(payer, token)
        .await
        .map_err(|_| invalid("invalid_exact_keeta_balance_check_failed"))?;
    if balance < *required_amount {
        return Err(invalid("invalid_exact_keeta_payload_insufficient_funds"));
    }

    let head = preflight
        .head_hash(payer)
        .await
        .map_err(|_| invalid("invalid_exact_keeta_state_check_failed"))?;
    let expected = head.unwrap_or_else(|| block.data().account().to_opening_hash());
    if *block.data().previous() != expected {
        return Err(invalid(
            "invalid_exact_keeta_payload_previous_head_mismatch",
        ));
    }

    let signer = block_signer_address(block);
    if signer != payer {
        let acls = preflight
            .acls_for_signer(&signer)
            .await
            .map_err(|_| invalid("invalid_exact_keeta_permission_check_failed"))?;
        let allowed = acls
            .iter()
            .any(|acl| acl.entity == payer && (acl.owner || acl.send_on_behalf));
        if !allowed {
            return Err(invalid("invalid_exact_keeta_payload_missing_permission"));
        }
    }
    Ok(())
}

fn block_signer_address(block: &Block) -> String {
    block
        .data()
        .signer()
        .required_signers()
        .first()
        .map_or_else(
            || block.data().signer().principal().to_string(),
            ToString::to_string,
        )
}

fn extra_external(requirements: &Value) -> Option<&str> {
    let external = requirements
        .get("extra")
        .and_then(|extra| extra.get("external"))
        .and_then(Value::as_str)?;
    if external.is_empty() {
        None
    } else {
        Some(external)
    }
}

fn parse_caip_network(network: &str) -> Result<(&str, u128), KeetaInvalid> {
    let Some((namespace, reference)) = network.split_once(':') else {
        return Err(invalid(
            "invalid_exact_keeta_requirements_network_malformed",
        ));
    };
    if reference.contains(':') {
        return Err(invalid(
            "invalid_exact_keeta_requirements_network_malformed",
        ));
    }
    let Some(id) = parse_network_id(reference) else {
        return Err(invalid("invalid_exact_keeta_requirements_network_id"));
    };
    Ok((namespace, id))
}

fn parse_network_id(s: &str) -> Option<u128> {
    if s.is_empty() || !s.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    s.parse().ok()
}

/// Decodes a base64 ASN.1 DER signed block (`Block::try_from`).
///
/// # Errors
///
/// Returns an error string when base64 or DER decoding / signature verify
/// fails.
pub fn decode_block(b64: &str) -> Result<Block, String> {
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(b64)
        .map_err(|e| e.to_string())?;
    Block::try_from(bytes.as_slice()).map_err(|e| e.to_string())
}
