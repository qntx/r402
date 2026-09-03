//! Dual-format tool results and `_meta` attach/extract.
//!
//! Payment-required: `isError` + `structuredContent` + `content[0].text`.

use r402_protocol::payment::{
    Base64Bytes, PaymentPayload, PaymentRequired, PaymentRequirements, SettleResponse,
};
use rmcp::model::{CallToolRequestParams, CallToolResult, ContentBlock, RequestMetaObject};
use serde_json::Value;

use crate::error::{McpCallError, McpClientError};
use crate::{
    JSONRPC_PAYMENT_REQUIRED_CODE, MCP_PAYMENT_META_KEY, MCP_PAYMENT_REQUIRED_CODE,
    MCP_PAYMENT_RESPONSE_META_KEY, MCP_TOOL_URL_PREFIX,
};

/// Wire payment payload shape used on MCP `_meta` (opaque scheme payload).
pub type McpPaymentPayload = PaymentPayload<PaymentRequirements, Value>;

/// Builds a tool resource URL: custom if non-empty, else `mcp://tool/{name}`.
#[must_use]
pub fn create_tool_resource_url(tool_name: &str, custom_url: Option<&str>) -> String {
    match custom_url {
        Some(url) if !url.is_empty() => url.to_owned(),
        _ => format!("{MCP_TOOL_URL_PREFIX}{tool_name}"),
    }
}

/// Payment-required tool result: dual `structuredContent` + `content[0].text`.
#[must_use]
pub fn payment_required_tool_result(required: &PaymentRequired) -> CallToolResult {
    let value = serde_json::to_value(required).unwrap_or(Value::Null);
    CallToolResult::structured_error(value)
}

/// Settlement-failed result (same dual format as payment-required).
#[must_use]
pub fn settlement_failed_tool_result(required: &PaymentRequired) -> CallToolResult {
    payment_required_tool_result(required)
}

/// Prefer `structuredContent`, else parse first text content as JSON.
///
/// Requires `x402Version` present and non-empty `accepts`; V2 only.
#[must_use]
pub fn extract_payment_required(result: &CallToolResult) -> Option<PaymentRequired> {
    if !result.is_error.unwrap_or(false) {
        return None;
    }
    if let Some(ref structured) = result.structured_content
        && let Some(required) = payment_required_from_value(structured)
    {
        return Some(required);
    }
    result
        .content
        .iter()
        .find_map(ContentBlock::as_text)
        .and_then(|text| serde_json::from_str::<Value>(&text.text).ok())
        .and_then(|value| payment_required_from_value(&value))
}

fn payment_required_from_value(value: &Value) -> Option<PaymentRequired> {
    let object = value.as_object()?;
    if !object.contains_key("x402Version") || !object.contains_key("accepts") {
        return None;
    }
    let version = object.get("x402Version").and_then(Value::as_u64)?;
    if version != 2 {
        return None;
    }
    let accepts = object.get("accepts")?.as_array()?;
    if accepts.is_empty() {
        return None;
    }
    serde_json::from_value(value.clone()).ok()
}

/// Reads `_meta["x402/payment"]` from tool-call params. Invalid structure → `None`.
#[must_use]
pub fn extract_payment_from_params(params: &CallToolRequestParams) -> Option<McpPaymentPayload> {
    let meta = params.meta.as_ref()?;
    let value = meta.get(MCP_PAYMENT_META_KEY)?.clone();
    let payload: McpPaymentPayload = serde_json::from_value(value).ok()?;
    if payload.payload.is_null() {
        return None;
    }
    Some(payload)
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

/// Extracts V2 [`PaymentRequired`] from a JSON-RPC payment challenge.
///
/// Code `402` uses `data`; `-32042` uses `data` or `data.x402`.
#[must_use]
pub fn extract_payment_required_from_rpc(err: &McpCallError) -> Option<PaymentRequired> {
    let McpCallError::Rpc { code, data, .. } = err else {
        return None;
    };
    let data = data.as_ref()?;
    match *code {
        MCP_PAYMENT_REQUIRED_CODE => payment_required_from_value(data),
        JSONRPC_PAYMENT_REQUIRED_CODE => payment_required_from_value(data)
            .or_else(|| data.get("x402").and_then(payment_required_from_value)),
        _ => None,
    }
}

/// True when a JSON-RPC error is an x402 payment-required challenge.
#[must_use]
pub fn is_payment_required_rpc(err: &McpCallError) -> bool {
    extract_payment_required_from_rpc(err).is_some()
}

/// Decodes a [`r402_client::CreatedPayment::signed_payload`] into MCP meta JSON.
///
/// # Errors
///
/// Base64 or JSON decode failure.
pub fn payment_payload_from_signed(signed: &str) -> Result<McpPaymentPayload, McpClientError> {
    let decoded = Base64Bytes::from(signed.as_bytes())
        .decode()
        .map_err(|err| McpClientError::Payment(err.to_string()))?;
    serde_json::from_slice(&decoded).map_err(|err| McpClientError::Payment(err.to_string()))
}
