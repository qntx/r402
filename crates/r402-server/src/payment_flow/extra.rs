//! `extra.paymentFlow` / `extra.assetTransferMethod` wire helpers.

use r402_protocol::payment::PaymentRequirements;
use serde_json::{Map, Value};

use super::{PaymentFlowName, ResolvedPaymentFlow, SDK_DEFAULT_ASSET_TRANSFER_METHOD};

/// `extra.paymentFlow` when `extra` is a JSON object.
#[must_use]
pub fn extra_payment_flow(extra: Option<&Value>) -> Option<&Value> {
    extra.and_then(|value| value.get("paymentFlow"))
}

/// Whether `extra.paymentFlow` is absent/null or a closed payment-flow name.
#[must_use]
pub fn is_recognized_payment_flow(extra: Option<&Value>) -> bool {
    match extra_payment_flow(extra) {
        None | Some(Value::Null) => true,
        Some(Value::String(name)) => name.parse::<PaymentFlowName>().is_ok(),
        Some(_) => false,
    }
}

/// Whether the accept is authorization (omit, JSON null, or `"authorization"`).
#[must_use]
pub fn is_authorization_payment_flow(extra: Option<&Value>) -> bool {
    match extra_payment_flow(extra) {
        None | Some(Value::Null) => true,
        Some(Value::String(name)) => name == PaymentFlowName::Authorization.as_str(),
        Some(_) => false,
    }
}

/// Applies resolved payment-flow rules to 402 `extra`:
///
/// - Strip the SDK ATM sentinel `"default"` (never on the wire).
/// - When the resolved flow is not `authorization`, set `extra.paymentFlow`.
#[must_use]
pub fn apply_payment_flow_wire_extra(
    extra: Option<&Value>,
    resolved: &ResolvedPaymentFlow,
) -> Map<String, Value> {
    let mut next = extra
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    let extra_atm = next.get("assetTransferMethod").and_then(Value::as_str);
    if resolved.asset_transfer_method == SDK_DEFAULT_ASSET_TRANSFER_METHOD
        || extra_atm == Some(SDK_DEFAULT_ASSET_TRANSFER_METHOD)
    {
        next.remove("assetTransferMethod");
    }
    if resolved.payment_flow != PaymentFlowName::Authorization {
        next.insert(
            "paymentFlow".to_owned(),
            Value::String(resolved.payment_flow.as_str().to_owned()),
        );
    }
    next
}

pub(super) fn extra_string<'a>(
    requirements: &'a PaymentRequirements,
    key: &str,
) -> Option<&'a str> {
    requirements
        .extra
        .as_ref()
        .and_then(|extra| extra.get(key))
        .and_then(Value::as_str)
}

pub(super) fn requested_flow(requirements: &PaymentRequirements) -> Option<String> {
    let value = requirements
        .extra
        .as_ref()
        .and_then(|extra| extra.get("paymentFlow"))?;
    match value {
        Value::Null => None,
        Value::String(s) => Some(s.clone()),
        other => Some(other.to_string()),
    }
}
