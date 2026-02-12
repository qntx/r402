//! Utility functions for extracting and attaching x402 payment data in MCP `_meta` fields.
//!
//! These functions work with [`serde_json::Value`] maps, making them
//! framework-agnostic and compatible with any MCP SDK implementation.

use r402::proto;
use serde_json::Value;

use crate::types::{CallToolResult, ContentItem};
use crate::{PAYMENT_META_KEY, PAYMENT_RESPONSE_META_KEY};

/// Extracts an x402 payment payload from an MCP request's `_meta` field.
///
/// Returns `None` if no payment is present or the data is malformed.
///
/// # Examples
///
/// ```
/// use r402_mcp::extract::extract_payment_from_meta;
///
/// let meta = serde_json::Map::new();
/// assert!(extract_payment_from_meta(&meta).is_none());
/// ```
#[must_use]
pub fn extract_payment_from_meta(meta: &serde_json::Map<String, Value>) -> Option<Value> {
    let payment = meta.get(PAYMENT_META_KEY)?;

    // Validate basic structure: must have x402Version and payload
    let obj = payment.as_object()?;
    let version = obj.get("x402Version")?.as_u64()?;
    if version == 0 {
        return None;
    }
    obj.get("payload")?;

    Some(payment.clone())
}

/// Attaches an x402 payment payload to an MCP request's `_meta` field.
///
/// Creates the `_meta` map if it doesn't exist. Overwrites any existing
/// payment data under the [`PAYMENT_META_KEY`].
pub fn attach_payment_to_meta(meta: &mut serde_json::Map<String, Value>, payment: Value) {
    meta.insert(PAYMENT_META_KEY.to_owned(), payment);
}

/// Extracts an x402 settlement response from an MCP result's `_meta` field.
///
/// Returns `None` if no settlement response is present or deserialization fails.
#[must_use]
pub fn extract_payment_response_from_meta(
    meta: &serde_json::Map<String, Value>,
) -> Option<proto::SettleResponse> {
    let response_data = meta.get(PAYMENT_RESPONSE_META_KEY)?;
    serde_json::from_value(response_data.clone()).ok()
}

/// Attaches an x402 settlement response to an MCP result's `_meta` field.
///
/// Creates the `_meta` map if it doesn't exist.
///
/// # Errors
///
/// Returns `Err` if the settlement response cannot be serialized.
pub fn attach_payment_response_to_meta(
    meta: &mut serde_json::Map<String, Value>,
    response: &proto::SettleResponse,
) -> Result<(), serde_json::Error> {
    let value = serde_json::to_value(response)?;
    meta.insert(PAYMENT_RESPONSE_META_KEY.to_owned(), value);
    Ok(())
}

/// Extracts a [`proto::PaymentRequired`] from an MCP tool error result.
///
/// Follows the MCP x402 specification for extracting payment required data:
/// 1. Checks `structuredContent` first (preferred path)
/// 2. Falls back to parsing `content[0].text` as JSON
///
/// Returns `None` if the result is not an error or contains no payment required data.
#[must_use]
pub fn extract_payment_required_from_result(
    result: &CallToolResult,
) -> Option<proto::PaymentRequired> {
    if !result.is_error {
        return None;
    }

    // Preferred path: structuredContent
    if let Some(sc) = &result.structured_content
        && let Some(pr) = try_parse_payment_required_from_value(sc)
    {
        return Some(pr);
    }

    // Fallback: parse content[0].text as JSON
    for item in &result.content {
        let ContentItem::Text { text } = item;
        if let Some(pr) = try_parse_payment_required_from_text(text) {
            return Some(pr);
        }
    }

    None
}

/// Creates a resource URL for an MCP tool.
///
/// If `custom_url` is provided, returns it directly.
/// Otherwise, generates a default `mcp://tool/<tool_name>` URL.
#[must_use]
pub fn create_tool_resource_url(tool_name: &str, custom_url: Option<&str>) -> String {
    custom_url.map_or_else(|| format!("mcp://tool/{tool_name}"), str::to_owned)
}

/// Attempts to parse a [`proto::PaymentRequired`] from a JSON value.
///
/// Validates that `x402Version` and `accepts` fields are present before
/// attempting deserialization.
fn try_parse_payment_required_from_value(value: &Value) -> Option<proto::PaymentRequired> {
    let obj = value.as_object()?;

    // Must have x402Version (numeric, >= 1)
    let version = obj.get("x402Version")?;
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let version_num = version.as_u64().or_else(|| {
        version.as_f64().and_then(|f| {
            if f >= 1.0 && f <= f64::from(u32::MAX) {
                Some(f as u64)
            } else {
                None
            }
        })
    })?;
    if version_num < 1 {
        return None;
    }

    // Must have non-empty accepts array
    let accepts = obj.get("accepts")?.as_array()?;
    if accepts.is_empty() {
        return None;
    }

    serde_json::from_value(value.clone()).ok()
}

/// Attempts to parse a [`proto::PaymentRequired`] from a JSON text string.
fn try_parse_payment_required_from_text(text: &str) -> Option<proto::PaymentRequired> {
    let value: Value = serde_json::from_str(text).ok()?;
    try_parse_payment_required_from_value(&value)
}
