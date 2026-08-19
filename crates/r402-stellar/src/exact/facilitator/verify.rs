//! Facilitator verification for the Stellar exact scheme.

use compact_str::CompactString;
use r402_core::wire::VerifyResponse;
use serde_json::Value;
use stellar_rpc_client::SimulateTransactionResponse;
use stellar_xdr::{SorobanCredentials, TransactionEnvelope};

use crate::chain::rpc::StellarRpc;
use crate::chain::types::{ed25519_account_payload, is_facilitator_account, is_stellar_network};
use crate::chain::xdr::{
    StellarXdrError, auth_entry_address, decode_transaction_envelope,
    gather_auth_entry_signature_status, inner_transaction, invoke_host_function_op,
    muxed_account_ed25519, parse_transfer_event, parse_transfer_invocation,
};
use crate::exact::StellarExtra;
use crate::exact::error::{StellarInvalid, invalid};
use crate::{
    BASE_FEE_STROOPS, DEFAULT_TIMEOUT_SECONDS, SIGNATURE_EXPIRATION_LEDGER_TOLERANCE,
    timeout_ledgers,
};

/// Successful verification plus the simulation used at settle time.
#[derive(Debug, Clone)]
pub struct VerifiedStellar {
    /// Transfer `from` address.
    pub payer: CompactString,
    /// Decoded client envelope.
    pub envelope: TransactionEnvelope,
    /// Simulation response (success, no restore).
    pub simulation: SimulateTransactionResponse,
}

/// Verifies a raw facilitator verify/settle request JSON.
pub async fn verify_request_json<R: StellarRpc>(
    rpc: &R,
    facilitator_addresses: &[String],
    max_transaction_fee_stroops: u32,
    request: &Value,
) -> VerifyResponse {
    match verify_inner(
        rpc,
        facilitator_addresses,
        max_transaction_fee_stroops,
        request,
    )
    .await
    {
        Ok(verified) => VerifyResponse::valid(verified.payer),
        Err(invalid) => invalid.into_response(),
    }
}

/// Verifies and returns the decoded envelope + simulation for settlement.
///
/// # Errors
///
/// Returns [`StellarInvalid`] when the payload fails the Stellar exact
/// verification checklist.
pub async fn verify_for_settle<R: StellarRpc>(
    rpc: &R,
    facilitator_addresses: &[String],
    max_transaction_fee_stroops: u32,
    request: &Value,
) -> Result<VerifiedStellar, StellarInvalid> {
    verify_inner(
        rpc,
        facilitator_addresses,
        max_transaction_fee_stroops,
        request,
    )
    .await
}

#[allow(
    clippy::too_many_lines,
    clippy::cognitive_complexity,
    reason = "sequential TS verify checklist must stay in one function"
)]
async fn verify_inner<R: StellarRpc>(
    rpc: &R,
    facilitator_addresses: &[String],
    max_transaction_fee_stroops: u32,
    request: &Value,
) -> Result<VerifiedStellar, StellarInvalid> {
    let payload = request
        .get("paymentPayload")
        .ok_or_else(|| invalid("invalid_payload"))?;
    let requirements = request
        .get("paymentRequirements")
        .ok_or_else(|| invalid("invalid_payment_requirements"))?;

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
    if accepted_network != req_network {
        return Err(invalid("network_mismatch"));
    }
    if !is_stellar_network(req_network) {
        return Err(invalid("invalid_network"));
    }
    if !extra_is_sponsored(requirements.get("extra")) || !extra_is_sponsored(accepted.get("extra"))
    {
        return Err(invalid("invalid_exact_stellar_fees_sponsored_unsupported"));
    }

    let tx_b64 = payload
        .get("payload")
        .and_then(|p| p.get("transaction"))
        .and_then(Value::as_str);
    let Some(tx_b64) = tx_b64 else {
        return Err(invalid("invalid_exact_stellar_payload_malformed"));
    };
    let envelope = decode_transaction_envelope(tx_b64)
        .map_err(|_| invalid("invalid_exact_stellar_payload_malformed"))?;
    let tx = inner_transaction(&envelope)
        .map_err(|_| invalid("invalid_exact_stellar_payload_malformed"))?;

    let Ok(invoke) = invoke_host_function_op(tx) else {
        return Err(invalid("invalid_exact_stellar_payload_wrong_operation"));
    };

    if is_facilitator_muxed(facilitator_addresses, &tx.source_account) {
        return Err(invalid(
            "invalid_exact_stellar_payload_unsafe_tx_or_op_source",
        ));
    }
    if let Some(op_source) = tx
        .operations
        .first()
        .and_then(|op| op.source_account.as_ref())
        && is_facilitator_muxed(facilitator_addresses, op_source)
    {
        return Err(invalid(
            "invalid_exact_stellar_payload_unsafe_tx_or_op_source",
        ));
    }

    let invocation = match parse_transfer_invocation(&invoke.host_function) {
        Ok(inv) => inv,
        Err(StellarXdrError::Shape(msg)) if msg.contains("transfer") => {
            return Err(invalid("invalid_exact_stellar_payload_wrong_function_name"));
        }
        Err(_) => return Err(invalid("invalid_exact_stellar_payload_wrong_operation")),
    };

    let asset = requirements
        .get("asset")
        .and_then(Value::as_str)
        .unwrap_or("");
    if invocation.asset != asset {
        return Err(invalid("invalid_exact_stellar_payload_wrong_asset"));
    }

    let from = CompactString::from(invocation.from.as_str());
    if is_facilitator_account(facilitator_addresses, from.as_str()) {
        return Err(invalid(
            "invalid_exact_stellar_payload_facilitator_is_payer",
        ));
    }

    let pay_to = requirements
        .get("payTo")
        .and_then(Value::as_str)
        .unwrap_or("");
    if invocation.to != pay_to {
        return Err(
            invalid("invalid_exact_stellar_payload_wrong_recipient").with_payer(from.clone())
        );
    }

    let expected_amount = requirements
        .get("amount")
        .and_then(Value::as_str)
        .and_then(|s| s.parse::<i128>().ok());
    if expected_amount != Some(invocation.amount) {
        return Err(invalid("invalid_exact_stellar_payload_wrong_amount").with_payer(from.clone()));
    }

    let simulation = rpc.simulate_transaction(&envelope).await.map_err(|_| {
        invalid("invalid_exact_stellar_payload_simulation_failed").with_payer(from.clone())
    })?;
    if simulation.error.is_some() || simulation.restore_preamble.is_some() {
        return Err(
            invalid("invalid_exact_stellar_payload_simulation_failed").with_payer(from.clone())
        );
    }

    let settlement_fee = simulation
        .min_resource_fee
        .saturating_add(u64::from(BASE_FEE_STROOPS));
    if settlement_fee > u64::from(max_transaction_fee_stroops) {
        return Err(
            invalid("invalid_exact_stellar_payload_fee_exceeds_maximum")
                .with_payer(from.clone())
                .with_message(format!(
                    "simulation-derived fee {settlement_fee} stroops exceeds ceiling {max_transaction_fee_stroops} stroops"
                )),
        );
    }

    let events = simulation
        .events()
        .map_err(|_| invalid("unexpected_verify_error").with_payer(from.clone()))?;
    validate_simulation_events(&events, from.as_str(), pay_to, invocation.amount, asset)?;

    let current_ledger = rpc
        .latest_ledger()
        .await
        .map_err(|_| invalid("unexpected_verify_error").with_payer(from.clone()))?;
    let estimated = rpc.estimated_ledger_seconds().await;
    let max_timeout = requirements
        .get("maxTimeoutSeconds")
        .and_then(Value::as_u64)
        .filter(|v| *v > 0)
        .unwrap_or(DEFAULT_TIMEOUT_SECONDS);
    let max_ledger = current_ledger.saturating_add(timeout_ledgers(max_timeout, estimated));
    validate_auth_entries(
        &invoke.auth,
        facilitator_addresses,
        from.as_str(),
        max_ledger,
    )?;

    Ok(VerifiedStellar {
        payer: from,
        envelope,
        simulation,
    })
}

fn extra_is_sponsored(extra: Option<&Value>) -> bool {
    extra
        .and_then(|v| serde_json::from_value::<StellarExtra>(v.clone()).ok())
        .is_some_and(|e| e.are_fees_sponsored)
}

fn is_facilitator_muxed(
    facilitator_addresses: &[String],
    account: &stellar_xdr::MuxedAccount,
) -> bool {
    let key = muxed_account_ed25519(account);
    facilitator_addresses
        .iter()
        .any(|addr| ed25519_account_payload(addr) == Some(key))
}

fn validate_simulation_events(
    events: &[stellar_xdr::DiagnosticEvent],
    from: &str,
    to: &str,
    expected_amount: i128,
    expected_asset: &str,
) -> Result<(), StellarInvalid> {
    let mut transfers = Vec::new();
    for event in events {
        match parse_transfer_event(event) {
            Ok(None) => {}
            Ok(Some(transfer)) => {
                if transfer.asset != expected_asset {
                    return Err(
                        invalid("invalid_exact_stellar_payload_event_wrong_asset").with_payer(from)
                    );
                }
                transfers.push(transfer);
            }
            Err(StellarXdrError::Shape(msg)) if msg.contains("contract id") => {
                return Err(
                    invalid("invalid_exact_stellar_payload_event_missing_contract_id")
                        .with_payer(from),
                );
            }
            Err(_) => {
                return Err(
                    invalid("invalid_exact_stellar_payload_event_not_transfer").with_payer(from)
                );
            }
        }
    }
    if transfers.is_empty() {
        return Err(invalid("invalid_exact_stellar_payload_no_transfer_events").with_payer(from));
    }
    if transfers.len() > 1 {
        return Err(invalid("invalid_exact_stellar_payload_multiple_transfers").with_payer(from));
    }
    let transfer = transfers.first().ok_or_else(|| {
        invalid("invalid_exact_stellar_payload_no_transfer_events").with_payer(from)
    })?;
    if transfer.from != from {
        return Err(invalid("invalid_exact_stellar_payload_event_wrong_from").with_payer(from));
    }
    if transfer.to != to {
        return Err(invalid("invalid_exact_stellar_payload_event_wrong_to").with_payer(from));
    }
    if transfer.amount != expected_amount {
        return Err(invalid("invalid_exact_stellar_payload_event_wrong_amount").with_payer(from));
    }
    Ok(())
}

fn validate_auth_entries(
    auth: &[stellar_xdr::SorobanAuthorizationEntry],
    facilitator_addresses: &[String],
    from: &str,
    max_ledger: u32,
) -> Result<(), StellarInvalid> {
    if auth.is_empty() {
        return Err(invalid("invalid_exact_stellar_payload_no_auth_entries").with_payer(from));
    }
    for entry in auth {
        if !matches!(entry.credentials, SorobanCredentials::Address(_)) {
            return Err(
                invalid("invalid_exact_stellar_payload_unsupported_credential_type")
                    .with_payer(from),
            );
        }
        if let Some(address) = auth_entry_address(entry)
            && is_facilitator_account(facilitator_addresses, &address)
        {
            return Err(
                invalid("invalid_exact_stellar_payload_facilitator_in_auth").with_payer(from)
            );
        }
        if let SorobanCredentials::Address(creds) = &entry.credentials {
            let limit = max_ledger.saturating_add(SIGNATURE_EXPIRATION_LEDGER_TOLERANCE);
            if creds.signature_expiration_ledger > limit {
                return Err(
                    invalid("invalid_exact_stellar_signature_expiration_too_far").with_payer(from),
                );
            }
        }
        if !entry.root_invocation.sub_invocations.is_empty() {
            return Err(
                invalid("invalid_exact_stellar_payload_has_subinvocations").with_payer(from)
            );
        }
    }
    let status = gather_auth_entry_signature_status(auth);
    if !status.already_signed.iter().any(|a| a == from) {
        return Err(
            invalid("invalid_exact_stellar_payload_missing_payer_signature").with_payer(from),
        );
    }
    if !status.pending_signature.is_empty() {
        return Err(
            invalid("invalid_exact_stellar_payload_unexpected_pending_signatures").with_payer(from),
        );
    }
    Ok(())
}
