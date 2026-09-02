//! HTTP header names and Cache-Control helpers for x402.
//!
//! Request: [`PAYMENT_SIGNATURE`], [`SIGN_IN_WITH_X`] (never emit as response).
//! Response: [`PAYMENT_REQUIRED`], [`PAYMENT_RESPONSE`].
//! Facilitator-only: [`EXTENSION_RESPONSES`] (not buyer CORS).

use http::{HeaderMap, HeaderValue, header};

/// Request header carrying a base64 `PaymentPayload`.
pub const PAYMENT_SIGNATURE: &str = "Payment-Signature";
/// Response header carrying a base64 `PaymentRequired` challenge.
pub const PAYMENT_REQUIRED: &str = "Payment-Required";
/// Response header carrying a base64 `SettleResponse` receipt.
pub const PAYMENT_RESPONSE: &str = "Payment-Response";
/// Request header carrying a base64 SIWX proof. Never emit as a response header.
pub const SIGN_IN_WITH_X: &str = "SIGN-IN-WITH-X";
/// Facilitator-only sidechannel. Not a buyer CORS expose/allow header.
pub const EXTENSION_RESPONSES: &str = "EXTENSION-RESPONSES";
/// CORS `Access-Control-Expose-Headers` value (`Payment-*` only).
pub const X402_EXPOSED_HEADERS: &str = "Payment-Required, Payment-Response";
/// CORS `Access-Control-Allow-Headers` value for the app `CorsLayer`.
pub const X402_ALLOW_HEADERS: &str = "Payment-Signature, SIGN-IN-WITH-X";

/// Sets `Cache-Control: no-store` (402 / 412 / settle-failure).
pub fn set_no_store(headers: &mut HeaderMap) {
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
}

/// Merges `private` into `Cache-Control` (200 with `Payment-Response`).
///
/// Missing or empty `Cache-Control` becomes `private`. An existing value that
/// does not already contain `private` becomes `existing, private`.
pub fn merge_private(headers: &mut HeaderMap) {
    match headers
        .get(header::CACHE_CONTROL)
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
    {
        None | Some("") => {
            headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("private"));
        }
        Some(existing) if cache_directive_present(existing, "private") => {}
        Some(existing) => {
            let merged = format!("{existing}, private");
            if let Ok(value) = HeaderValue::from_str(&merged) {
                headers.insert(header::CACHE_CONTROL, value);
            }
        }
    }
}

fn cache_directive_present(header: &str, token: &str) -> bool {
    header
        .split(',')
        .any(|part| part.trim().eq_ignore_ascii_case(token))
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "unit tests panic on assertion failure"
)]
mod tests {
    use super::*;

    #[test]
    fn header_names_match_contract() {
        assert_eq!(PAYMENT_SIGNATURE, "Payment-Signature");
        assert_eq!(PAYMENT_REQUIRED, "Payment-Required");
        assert_eq!(PAYMENT_RESPONSE, "Payment-Response");
        assert_eq!(SIGN_IN_WITH_X, "SIGN-IN-WITH-X");
        assert_eq!(EXTENSION_RESPONSES, "EXTENSION-RESPONSES");
        assert_eq!(X402_EXPOSED_HEADERS, "Payment-Required, Payment-Response");
        assert_eq!(X402_ALLOW_HEADERS, "Payment-Signature, SIGN-IN-WITH-X");
    }

    #[test]
    fn set_no_store_overwrites_existing() {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::CACHE_CONTROL,
            HeaderValue::from_static("max-age=60"),
        );
        set_no_store(&mut headers);
        assert_eq!(
            headers.get(header::CACHE_CONTROL).unwrap(),
            "no-store",
            "402/412 Cache-Control is no-store, not merged"
        );
    }

    #[test]
    fn merge_private_sets_appends_and_is_idempotent() {
        let mut headers = HeaderMap::new();
        merge_private(&mut headers);
        assert_eq!(headers.get(header::CACHE_CONTROL).unwrap(), "private");

        headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("max-age=0"));
        merge_private(&mut headers);
        assert_eq!(
            headers.get(header::CACHE_CONTROL).unwrap(),
            "max-age=0, private"
        );

        merge_private(&mut headers);
        assert_eq!(
            headers.get(header::CACHE_CONTROL).unwrap(),
            "max-age=0, private",
            "private must not be appended twice"
        );
    }
}
