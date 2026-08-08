//! Transport encoding for MCP × x402.
//!
//! Implements `specs/transports-v2/mcp.md` and Go `go/mcp/utils.go`:
//! payment-required results carry **both** `structuredContent` and
//! `content[0].text` (JSON string of the same object).

use r402_core::wire::{PaymentPayload, PaymentRequired, PaymentRequirements, SettleResponse};
use rmcp::model::{CallToolRequestParams, CallToolResult, ContentBlock, RequestMetaObject};
use serde_json::Value;

use crate::constants::{
    MCP_PAYMENT_META_KEY, MCP_PAYMENT_RESPONSE_META_KEY, MCP_TOOL_URL_PREFIX,
};

/// Wire payment payload shape used on MCP `_meta` (opaque scheme payload).
pub type McpPaymentPayload = PaymentPayload<PaymentRequirements, Value>;

/// Builds a tool resource URL: custom if non-empty, else `mcp://tool/{name}`.
#[must_use]
pub fn create_tool_resource_url(tool_name: &str, custom_url: Option<&str>) -> String {
    match custom_url {
        Some(u) if !u.is_empty() => u.to_owned(),
        _ => format!("{MCP_TOOL_URL_PREFIX}{tool_name}"),
    }
}

/// Payment-required tool result per transport spec.
///
/// Sets `isError: true`, `structuredContent` = `PaymentRequired`, and
/// `content[0].text` = JSON string of the same object
/// (`CallToolResult::structured_error` matches that shape).
#[must_use]
pub fn payment_required_tool_result(required: &PaymentRequired) -> CallToolResult {
    let value = serde_json::to_value(required).unwrap_or(Value::Null);
    CallToolResult::structured_error(value)
}

/// Settlement-failed tool result (handler succeeded but settle failed).
#[must_use]
pub fn settlement_failed_tool_result(message: impl Into<String>) -> CallToolResult {
    CallToolResult::error(vec![ContentBlock::text(message.into())])
}

/// Prefer `structuredContent`, else parse first text content as JSON.
#[must_use]
pub fn extract_payment_required(result: &CallToolResult) -> Option<PaymentRequired> {
    if !result.is_error.unwrap_or(false) {
        return None;
    }
    if let Some(ref sc) = result.structured_content
        && let Ok(pr) = serde_json::from_value::<PaymentRequired>(sc.clone())
        && !pr.accepts.is_empty()
    {
        return Some(pr);
    }
    result
        .content
        .first()
        .and_then(ContentBlock::as_text)
        .and_then(|t| serde_json::from_str(&t.text).ok())
        .filter(|pr: &PaymentRequired| !pr.accepts.is_empty())
}

/// Reads `_meta["x402/payment"]` from tool-call params.
#[must_use]
pub fn extract_payment_from_params(params: &CallToolRequestParams) -> Option<McpPaymentPayload> {
    let meta = params.meta.as_ref()?;
    let value = meta.get(MCP_PAYMENT_META_KEY)?.clone();
    serde_json::from_value(value).ok()
}

/// Writes `_meta["x402/payment"]` onto tool-call params.
#[must_use]
pub fn attach_payment_to_params(
    mut params: CallToolRequestParams,
    payload: &McpPaymentPayload,
) -> CallToolRequestParams {
    let value = serde_json::to_value(payload).unwrap_or(Value::Null);
    let meta = params.meta.get_or_insert_with(RequestMetaObject::new);
    meta.insert(MCP_PAYMENT_META_KEY.to_owned(), value);
    params
}

/// Reads settlement response from result `_meta`.
#[must_use]
pub fn extract_settle_response(result: &CallToolResult) -> Option<SettleResponse> {
    let meta = result.meta.as_ref()?;
    let value = meta.get(MCP_PAYMENT_RESPONSE_META_KEY)?.clone();
    serde_json::from_value(value).ok()
}

/// Attaches settlement response under `_meta["x402/payment-response"]`.
#[must_use]
pub fn attach_settle_response(
    mut result: CallToolResult,
    settle: &SettleResponse,
) -> CallToolResult {
    let value = serde_json::to_value(settle).unwrap_or(Value::Null);
    let mut meta = result.meta.take().unwrap_or_default();
    meta.insert(MCP_PAYMENT_RESPONSE_META_KEY.to_owned(), value);
    result.meta = Some(meta);
    result
}

/// True when the tool result is an x402 payment-required challenge.
#[must_use]
pub fn is_payment_required_result(result: &CallToolResult) -> bool {
    extract_payment_required(result).is_some()
}

#[cfg(test)]
mod tests {
    use r402_core::wire::{PaymentRequirements, ResourceInfo};

    use super::*;

    fn sample_required() -> PaymentRequired {
        let resource = ResourceInfo::new("mcp://tool/demo");
        let req = PaymentRequirements::new(
            "exact".into(),
            "eip155:84532".parse().unwrap(),
            "10000".into(),
            "0xpay".into(),
            "0xasset".into(),
            60,
        );
        PaymentRequired::new(resource)
            .with_error("Payment required")
            .with_accepts(vec![req])
    }

    #[test]
    fn payment_required_sets_structured_and_text() {
        let pr = sample_required();
        let result = payment_required_tool_result(&pr);
        assert_eq!(result.is_error, Some(true));
        assert!(result.structured_content.is_some());
        let text = result
            .content
            .first()
            .and_then(ContentBlock::as_text)
            .unwrap();
        let parsed: PaymentRequired = serde_json::from_str(&text.text).unwrap();
        assert_eq!(parsed.accepts.len(), 1);
        let again = extract_payment_required(&result).unwrap();
        assert_eq!(
            again.accepts.first().map(|a| a.amount.as_str()),
            Some("10000")
        );
    }

    #[test]
    fn tool_url_default_and_custom() {
        assert_eq!(
            create_tool_resource_url("weather", None),
            "mcp://tool/weather"
        );
        assert_eq!(
            create_tool_resource_url("weather", Some("https://x/y")),
            "https://x/y"
        );
    }

    #[test]
    fn attach_and_extract_payment_meta() {
        let payload: McpPaymentPayload = serde_json::from_value(serde_json::json!({
            "x402Version": 2,
            "accepted": {
                "scheme": "exact",
                "network": "eip155:1",
                "amount": "1",
                "payTo": "0xa",
                "asset": "0xb",
                "maxTimeoutSeconds": 60
            },
            "payload": {}
        }))
        .unwrap();
        let params = CallToolRequestParams::new("demo");
        let params = attach_payment_to_params(params, &payload);
        let extracted = extract_payment_from_params(&params).unwrap();
        assert_eq!(extracted.accepted.scheme.as_str(), "exact");
    }
}
