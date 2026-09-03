#![allow(unused_crate_dependencies, reason = "sibling tests consume them")]
#![allow(
    clippy::tests_outside_test_module,
    reason = "integration test binaries put #[test] fns at file scope"
)]
#![allow(
    clippy::doc_markdown,
    clippy::expect_used,
    clippy::panic,
    clippy::shadow_unrelated,
    clippy::unwrap_used,
    reason = "idiomatic test-code patterns"
)]

//! Header names, CORS expose, Cache-Control. Production constants vs `headers.json`.

mod harness;

use std::path::PathBuf;
use std::sync::Arc;

use http::StatusCode;
use http::header::{ACCESS_CONTROL_EXPOSE_HEADERS, CACHE_CONTROL};
use r402_http::headers::{
    EXTENSION_RESPONSES, PAYMENT_REQUIRED, PAYMENT_RESPONSE, PAYMENT_SIGNATURE, SIGN_IN_WITH_X,
    X402_EXPOSED_HEADERS,
};
use r402_http::{merge_private, set_no_store};
use serde_json::Value;

use crate::harness::{
    FakeFacilitator, FlowScheme, OkInner, call_layer, eip155_requirements, eip155_tag, middleware,
    payment_request, unpaid_request,
};

fn headers_fixture() -> Value {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/contract/headers.json");
    let bytes = std::fs::read(&path)
        .unwrap_or_else(|err| panic!("missing fixture {}: {err}", path.display()));
    serde_json::from_slice(&bytes)
        .unwrap_or_else(|err| panic!("invalid JSON in fixture {}: {err}", path.display()))
}

fn require<'a>(value: &'a Value, keys: &[&str]) -> &'a Value {
    let mut current = value;
    for key in keys {
        current = current
            .get(*key)
            .unwrap_or_else(|| panic!("missing key {} in headers.json", keys.join(".")));
    }
    current
}

fn require_str<'a>(value: &'a Value, keys: &[&str]) -> &'a str {
    require(value, keys)
        .as_str()
        .unwrap_or_else(|| panic!("{} must be a string", keys.join(".")))
}

fn require_u16(value: &Value, keys: &[&str]) -> u16 {
    require(value, keys)
        .as_u64()
        .and_then(|n| u16::try_from(n).ok())
        .unwrap_or_else(|| panic!("{} must be a u16 status", keys.join(".")))
}

#[test]
fn header_names() {
    let value = headers_fixture();
    assert_eq!(
        PAYMENT_SIGNATURE,
        require_str(&value, &["payment_signature"])
    );
    assert_eq!(PAYMENT_REQUIRED, require_str(&value, &["payment_required"]));
    assert_eq!(PAYMENT_RESPONSE, require_str(&value, &["payment_response"]));
    assert_eq!(SIGN_IN_WITH_X, require_str(&value, &["sign_in_with_x"]));
    assert_eq!(
        EXTENSION_RESPONSES,
        require_str(&value, &["extension_responses"])
    );
    assert_eq!(
        X402_EXPOSED_HEADERS,
        require_str(&value, &["expose_headers"])
    );
    assert_eq!(
        require_u16(&value, &["status", "unpaid"]),
        StatusCode::PAYMENT_REQUIRED.as_u16()
    );
    assert_eq!(
        require_u16(&value, &["status", "malformed_payment"]),
        StatusCode::PAYMENT_REQUIRED.as_u16()
    );
    assert_eq!(
        require_u16(&value, &["status", "permit2_allowance"]),
        StatusCode::PRECONDITION_FAILED.as_u16()
    );
    assert_eq!(
        require_u16(&value, &["status", "facilitator_transport"]),
        StatusCode::BAD_GATEWAY.as_u16()
    );
    assert_eq!(
        require_u16(&value, &["status", "missing_scheme"]),
        StatusCode::INTERNAL_SERVER_ERROR.as_u16()
    );
    assert_eq!(
        require_u16(&value, &["status", "incompatible_mode"]),
        StatusCode::INTERNAL_SERVER_ERROR.as_u16()
    );
    assert_eq!(
        require_u16(&value, &["status", "ok"]),
        StatusCode::OK.as_u16()
    );
    assert_eq!(
        require_str(&value, &["cache_control", "payment_required"]),
        "no-store"
    );
    assert_eq!(
        require_str(&value, &["cache_control", "settle_failure"]),
        "no-store"
    );
    assert_eq!(
        require_str(&value, &["cache_control", "ok_with_receipt"]),
        "private"
    );
}

#[tokio::test]
async fn unpaid_402_sets_no_store_and_cors() {
    let fac = Arc::new(FakeFacilitator::new());
    let layer = middleware(Arc::clone(&fac), FlowScheme::authorization())
        .with_price_tag(eip155_tag())
        .unwrap();
    let response = call_layer(layer, OkInner, unpaid_request()).await;
    assert_eq!(response.status(), StatusCode::PAYMENT_REQUIRED);
    assert_eq!(
        response
            .headers()
            .get(CACHE_CONTROL)
            .and_then(|v| v.to_str().ok()),
        Some("no-store")
    );
    let expose = response
        .headers()
        .get(ACCESS_CONTROL_EXPOSE_HEADERS)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default();
    assert!(expose.contains("Payment-Required"));
    assert!(expose.contains("Payment-Response"));
}

#[tokio::test]
async fn paid_200_merges_private() {
    let fac = Arc::new(FakeFacilitator::new());
    let layer = middleware(Arc::clone(&fac), FlowScheme::authorization())
        .with_price_tag(eip155_tag())
        .unwrap();
    let response = call_layer(layer, OkInner, payment_request(&eip155_requirements())).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response
            .headers()
            .get(CACHE_CONTROL)
            .and_then(|v| v.to_str().ok()),
        Some("private")
    );
}

#[test]
fn set_no_store_and_merge_private_helpers() {
    let mut headers = http::HeaderMap::new();
    set_no_store(&mut headers);
    assert_eq!(
        headers.get(CACHE_CONTROL).and_then(|v| v.to_str().ok()),
        Some("no-store")
    );
    let mut headers = http::HeaderMap::new();
    let _ = headers.insert(CACHE_CONTROL, http::HeaderValue::from_static("max-age=0"));
    merge_private(&mut headers);
    assert_eq!(
        headers.get(CACHE_CONTROL).and_then(|v| v.to_str().ok()),
        Some("max-age=0, private")
    );
}
