//! CORS helpers for exposing x402 response headers to browser clients.
//!
//! A browser-based buyer needs to read the `Payment-Required` and
//! `Payment-Response` headers from cross-origin responses. That read is
//! blocked by default unless the server explicitly advertises the headers
//! via `Access-Control-Expose-Headers`. [`ensure_expose_headers`] merges the
//! x402 entries into any existing value (idempotent).

use http::HeaderMap;
use http::header::{ACCESS_CONTROL_EXPOSE_HEADERS, HeaderValue};

/// The canonical list of x402 response headers clients need to see.
pub const X402_EXPOSED_HEADERS: &str = "Payment-Required, Payment-Response";

/// Ensures `Access-Control-Expose-Headers` on `headers` advertises the x402
/// response headers. Idempotent: calling more than once is a no-op.
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adds_header_when_absent() {
        let mut headers = HeaderMap::new();
        ensure_expose_headers(&mut headers);
        assert_eq!(
            headers.get(ACCESS_CONTROL_EXPOSE_HEADERS).unwrap(),
            X402_EXPOSED_HEADERS,
        );
    }

    #[test]
    fn merges_existing_header() {
        let mut headers = HeaderMap::new();
        let _ = headers.insert(ACCESS_CONTROL_EXPOSE_HEADERS, HeaderValue::from_static("X-Foo"));
        ensure_expose_headers(&mut headers);
        let value = headers.get(ACCESS_CONTROL_EXPOSE_HEADERS).unwrap();
        let value = value.to_str().unwrap();
        assert!(value.contains("X-Foo"));
        assert!(value.contains("Payment-Required"));
        assert!(value.contains("Payment-Response"));
    }

    #[test]
    fn idempotent_on_repeated_calls() {
        let mut headers = HeaderMap::new();
        ensure_expose_headers(&mut headers);
        ensure_expose_headers(&mut headers);
        let value = headers.get(ACCESS_CONTROL_EXPOSE_HEADERS).unwrap();
        assert_eq!(value, X402_EXPOSED_HEADERS);
    }
}
