//! x402 HTTP header names and Cache-Control helpers.
//!
//! Request: [`PAYMENT_SIGNATURE`], [`SIGN_IN_WITH_X`] (never emit as response).
//! Response: [`PAYMENT_REQUIRED`], [`PAYMENT_RESPONSE`].
//! Facilitator-only: [`EXTENSION_RESPONSES`] (not buyer CORS).

use http::header::{ACCESS_CONTROL_EXPOSE_HEADERS, CACHE_CONTROL};
use http::{HeaderMap, HeaderValue};

/// Buyer retry header carrying a base64 `PaymentPayload`.
pub const PAYMENT_SIGNATURE: &str = "Payment-Signature";

/// 402 challenge header carrying a base64 `PaymentRequired`.
pub const PAYMENT_REQUIRED: &str = "Payment-Required";

/// Settlement receipt header carrying a base64 `SettleResponse`.
pub const PAYMENT_RESPONSE: &str = "Payment-Response";

/// SIWX proof header (base64 JSON). Never emit as a response header.
pub const SIGN_IN_WITH_X: &str = "SIGN-IN-WITH-X";

/// Facilitator-only sidechannel. Not a buyer CORS expose/allow header.
pub const EXTENSION_RESPONSES: &str = "EXTENSION-RESPONSES";

/// CORS `Access-Control-Expose-Headers` value for x402 response headers.
pub const X402_EXPOSED_HEADERS: &str = "Payment-Required, Payment-Response";

/// CORS `Access-Control-Allow-Headers` value for the app `CorsLayer`.
pub const X402_ALLOW_HEADERS: &str = "Payment-Signature, SIGN-IN-WITH-X";

/// Ensures `Access-Control-Expose-Headers` advertises the x402 response headers.
pub fn ensure_expose_headers(headers: &mut HeaderMap) {
    let x402 = HeaderValue::from_static(X402_EXPOSED_HEADERS);
    match headers.get(ACCESS_CONTROL_EXPOSE_HEADERS) {
        None => {
            let _ = headers.insert(ACCESS_CONTROL_EXPOSE_HEADERS, x402);
        }
        Some(existing) => {
            let Ok(existing_str) = existing.to_str() else {
                let _ = headers.insert(ACCESS_CONTROL_EXPOSE_HEADERS, x402);
                return;
            };
            if existing_str.contains("Payment-Required")
                && existing_str.contains("Payment-Response")
            {
                return;
            }
            let merged = format!("{existing_str}, {X402_EXPOSED_HEADERS}");
            if let Ok(value) = HeaderValue::from_str(&merged) {
                let _ = headers.insert(ACCESS_CONTROL_EXPOSE_HEADERS, value);
            }
        }
    }
}

/// Writes `Cache-Control: no-store` (402 / 412 / settle-fail).
pub fn set_no_store(headers: &mut HeaderMap) {
    let _ = headers.insert(CACHE_CONTROL, HeaderValue::from_static("no-store"));
}

/// Merges `private` into `Cache-Control` (200 + `Payment-Response`).
///
/// Missing or empty `Cache-Control` becomes `private`. An existing value that
/// does not already contain `private` becomes `existing, private`.
pub fn merge_private(headers: &mut HeaderMap) {
    match headers
        .get(CACHE_CONTROL)
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
    {
        None | Some("") => {
            let _ = headers.insert(CACHE_CONTROL, HeaderValue::from_static("private"));
        }
        Some(existing) if cache_directive_present(existing, "private") => {}
        Some(existing) => {
            let merged = format!("{existing}, private");
            if let Ok(value) = HeaderValue::from_str(&merged) {
                let _ = headers.insert(CACHE_CONTROL, value);
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
mod tests {
    use http::header::{ACCESS_CONTROL_EXPOSE_HEADERS, CACHE_CONTROL};

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
    fn expose_headers_adds_when_absent() {
        let mut headers = HeaderMap::new();
        ensure_expose_headers(&mut headers);
        assert_eq!(
            headers
                .get(ACCESS_CONTROL_EXPOSE_HEADERS)
                .map(HeaderValue::as_bytes),
            Some(X402_EXPOSED_HEADERS.as_bytes()),
            "expose headers must be inserted when missing"
        );
    }

    #[test]
    fn expose_headers_merges_existing() {
        let mut headers = HeaderMap::new();
        let _ = headers.insert(
            ACCESS_CONTROL_EXPOSE_HEADERS,
            HeaderValue::from_static("X-Foo"),
        );
        ensure_expose_headers(&mut headers);
        let value = headers
            .get(ACCESS_CONTROL_EXPOSE_HEADERS)
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default();
        assert!(
            value.contains("X-Foo"),
            "existing CORS expose value must be kept"
        );
        assert!(
            value.contains("Payment-Required"),
            "Payment-Required must be exposed"
        );
        assert!(
            value.contains("Payment-Response"),
            "Payment-Response must be exposed"
        );
    }

    #[test]
    fn expose_headers_is_idempotent() {
        let mut headers = HeaderMap::new();
        ensure_expose_headers(&mut headers);
        ensure_expose_headers(&mut headers);
        assert_eq!(
            headers
                .get(ACCESS_CONTROL_EXPOSE_HEADERS)
                .map(HeaderValue::as_bytes),
            Some(X402_EXPOSED_HEADERS.as_bytes()),
            "second call must not duplicate expose headers"
        );
    }

    #[test]
    fn set_no_store_overwrites() {
        let mut headers = HeaderMap::new();
        let _ = headers.insert(CACHE_CONTROL, HeaderValue::from_static("max-age=60"));
        set_no_store(&mut headers);
        assert_eq!(
            headers.get(CACHE_CONTROL).map(HeaderValue::as_bytes),
            Some(b"no-store".as_slice()),
            "no-store must replace any prior Cache-Control"
        );
    }

    #[test]
    fn merge_private_when_absent() {
        let mut headers = HeaderMap::new();
        merge_private(&mut headers);
        assert_eq!(
            headers.get(CACHE_CONTROL).map(HeaderValue::as_bytes),
            Some(b"private".as_slice()),
            "missing Cache-Control must become private"
        );
    }

    #[test]
    fn merge_private_appends_existing() {
        let mut headers = HeaderMap::new();
        let _ = headers.insert(CACHE_CONTROL, HeaderValue::from_static("max-age=0"));
        merge_private(&mut headers);
        assert_eq!(
            headers.get(CACHE_CONTROL).and_then(|v| v.to_str().ok()),
            Some("max-age=0, private"),
            "existing Cache-Control must be suffixed with private"
        );
    }

    #[test]
    fn merge_private_is_idempotent() {
        let mut headers = HeaderMap::new();
        let _ = headers.insert(CACHE_CONTROL, HeaderValue::from_static("max-age=0"));
        merge_private(&mut headers);
        merge_private(&mut headers);
        assert_eq!(
            headers.get(CACHE_CONTROL).and_then(|v| v.to_str().ok()),
            Some("max-age=0, private"),
            "private must not be appended twice"
        );
    }
}
