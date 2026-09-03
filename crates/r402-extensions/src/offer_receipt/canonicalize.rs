//! JCS-style canonical JSON matching official offer-receipt `canonicalize`.

use serde::Serialize;
use serde_json::Value;

use super::OfferReceiptError;

/// Canonical JSON string (sorted object keys, no whitespace).
///
/// # Errors
///
/// [`OfferReceiptError::Canonicalize`] for non-finite numbers.
pub fn canonicalize(value: &Value) -> Result<String, OfferReceiptError> {
    serialize_value(value)
}

/// UTF-8 bytes of [`canonicalize`].
///
/// # Errors
///
/// [`OfferReceiptError::Canonicalize`].
pub fn canonicalize_to_bytes(value: &Value) -> Result<Vec<u8>, OfferReceiptError> {
    canonicalize(value).map(String::into_bytes)
}

/// Canonicalize any [`Serialize`] value via JSON.
///
/// # Errors
///
/// Serde or canonicalize failure.
pub(super) fn canonicalize_serialize<T: Serialize>(value: &T) -> Result<String, OfferReceiptError> {
    let json = serde_json::to_value(value)?;
    canonicalize(&json)
}

fn serialize_value(value: &Value) -> Result<String, OfferReceiptError> {
    match value {
        Value::Null => Ok("null".into()),
        Value::Bool(true) => Ok("true".into()),
        Value::Bool(false) => Ok("false".into()),
        Value::Number(n) => {
            if n.as_f64().is_some_and(|f| !f.is_finite()) {
                return Err(OfferReceiptError::Canonicalize(
                    "Cannot canonicalize Infinity or NaN".into(),
                ));
            }
            Ok(n.to_string())
        }
        Value::String(s) => Ok(serialize_string(s)),
        Value::Array(items) => {
            let mut parts = Vec::with_capacity(items.len());
            for item in items {
                parts.push(serialize_value(item)?);
            }
            Ok(format!("[{}]", parts.join(",")))
        }
        Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort_unstable();
            let mut pairs = Vec::with_capacity(keys.len());
            for key in keys {
                let Some(v) = map.get(key) else {
                    continue;
                };
                if v.is_null() && !map.contains_key(key) {
                    continue;
                }
                pairs.push(format!("{}:{}", serialize_string(key), serialize_value(v)?));
            }
            Ok(format!("{{{}}}", pairs.join(",")))
        }
    }
}

fn serialize_string(s: &str) -> String {
    let mut result = String::from("\"");
    for ch in s.chars() {
        let code = ch as u32;
        if code < 0x20 {
            result.push_str("\\u");
            let hex = format!("{code:04x}");
            result.push_str(&hex);
        } else if ch == '"' {
            result.push_str("\\\"");
        } else if ch == '\\' {
            result.push_str("\\\\");
        } else {
            result.push(ch);
        }
    }
    result.push('"');
    result
}
