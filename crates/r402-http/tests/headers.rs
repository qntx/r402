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

//! Header names, CORS expose, Cache-Control.

mod harness;

use std::sync::Arc;

use http::StatusCode;
use http::header::{ACCESS_CONTROL_EXPOSE_HEADERS, CACHE_CONTROL};
use r402_http::server::{PAYMENT_REQUIRED, PAYMENT_RESPONSE, PAYMENT_SIGNATURE, SIGN_IN_WITH_X};
use r402_http::{X402_EXPOSED_HEADERS, merge_private, set_no_store};

use crate::harness::{
    FakeFacilitator, FlowScheme, OkInner, call_layer, eip155_requirements, eip155_tag, middleware,
    payment_request, unpaid_request,
};

#[test]
fn header_names() {
    assert_eq!(PAYMENT_SIGNATURE, "Payment-Signature");
    assert_eq!(PAYMENT_REQUIRED, "Payment-Required");
    assert_eq!(PAYMENT_RESPONSE, "Payment-Response");
    assert_eq!(SIGN_IN_WITH_X, "SIGN-IN-WITH-X");
    assert_eq!(X402_EXPOSED_HEADERS, "Payment-Required, Payment-Response");
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
