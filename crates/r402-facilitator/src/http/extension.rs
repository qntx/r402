//! `EXTENSION-RESPONSES` header decode. Failure is ignored.

use http::HeaderMap;
use r402_protocol::payment::{Base64Bytes, Extensions, SettleResponse, VerifyResponse};

pub(super) fn attach_verify(mut response: VerifyResponse, headers: &HeaderMap) -> VerifyResponse {
    let side = parse_extension_responses(headers);
    log_extension_responses(&side);
    response.set_extension_responses(side);
    response
}

pub(super) fn attach_settle(mut response: SettleResponse, headers: &HeaderMap) -> SettleResponse {
    let side = parse_extension_responses(headers);
    log_extension_responses(&side);
    response.set_extension_responses(side);
    response
}

fn parse_extension_responses(headers: &HeaderMap) -> Extensions {
    let Some(value) = headers.get("extension-responses") else {
        return Extensions::new();
    };
    let Ok(raw) = value.to_str() else {
        return Extensions::new();
    };
    decode_extension_responses(raw).unwrap_or_default()
}

/// Decode failure is ignored: missing, bad base64, non-object JSON, or
/// deserialize error all yield an empty map.
fn decode_extension_responses(header: &str) -> Option<Extensions> {
    let bytes = Base64Bytes(header.as_bytes().to_vec());
    let decoded = bytes.decode().ok()?;
    let value: serde_json::Value = serde_json::from_slice(&decoded).ok()?;
    if !value.is_object() {
        return None;
    }
    serde_json::from_value(value).ok()
}

fn log_extension_responses(responses: &Extensions) {
    #[cfg(feature = "telemetry")]
    {
        if responses.is_empty() {
            return;
        }
        let sanitized: serde_json::Map<String, serde_json::Value> = responses
            .iter()
            .map(|(key, entry)| {
                (
                    key.to_string(),
                    serde_json::Value::Object(allowlisted(entry)),
                )
            })
            .collect();
        tracing::info!(
            extension_responses = %serde_json::Value::Object(sanitized),
            "x402.facilitator_client.extension_responses"
        );
    }
    #[cfg(not(feature = "telemetry"))]
    let _ = responses;
}

#[cfg(feature = "telemetry")]
fn allowlisted(
    entry: &r402_protocol::payment::ExtensionEntry,
) -> serde_json::Map<String, serde_json::Value> {
    use r402_protocol::payment::EXTENSION_RESPONSE_LOG_FIELDS;
    let Some(obj) = entry.to_value().as_object().cloned() else {
        return serde_json::Map::new();
    };
    EXTENSION_RESPONSE_LOG_FIELDS
        .iter()
        .filter_map(|field| obj.get(*field).cloned().map(|v| ((*field).to_owned(), v)))
        .collect()
}
