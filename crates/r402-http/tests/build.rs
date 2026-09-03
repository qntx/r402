#![allow(unused_crate_dependencies, reason = "sibling tests consume them")]
#![allow(
    clippy::tests_outside_test_module,
    reason = "integration test binaries put #[test] fns at file scope"
)]
#![allow(
    clippy::doc_markdown,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic,
    clippy::unwrap_used,
    reason = "idiomatic test-code patterns"
)]

//! Construct-time `MissingBaseUrl`. `with_resource` does not waive `base_url`.

mod harness;

use std::sync::Arc;

use http::StatusCode;
use r402_http::server::{BuildError, FacilitatorClientError, X402Middleware};

use crate::harness::{
    FakeFacilitator, FlowScheme, OkInner, base_url, call_layer, eip155_tag, unpaid_request,
};

#[test]
fn with_price_tag_requires_base_url() {
    let err = X402Middleware::from_facilitator(FakeFacilitator::new())
        .with_price_tag(eip155_tag())
        .expect_err("missing base_url must fail at construct time");
    assert_eq!(err, BuildError::MissingBaseUrl);
}

#[test]
fn with_price_tags_requires_base_url() {
    let err = X402Middleware::from_facilitator(FakeFacilitator::new())
        .with_price_tags(vec![eip155_tag()])
        .expect_err("missing base_url must fail at construct time");
    assert_eq!(err, BuildError::MissingBaseUrl);
}

#[test]
fn with_dynamic_price_requires_base_url() {
    let err = X402Middleware::from_facilitator(FakeFacilitator::new())
        .with_dynamic_price(|_, _, _| std::future::ready(vec![eip155_tag()]))
        .expect_err("missing base_url must fail at construct time");
    assert_eq!(err, BuildError::MissingBaseUrl);
}

#[test]
fn with_resource_does_not_waive_base_url() {
    // with_resource lives on X402Layer, which is unreachable without base_url.
    let err = X402Middleware::from_facilitator(FakeFacilitator::new()).with_price_tag(eip155_tag());
    assert!(
        matches!(err, Err(BuildError::MissingBaseUrl)),
        "with_resource cannot be called before with_price_tag, so it cannot waive base_url"
    );
}

#[test]
fn try_new_rejects_invalid_url() {
    let err = X402Middleware::try_new("not a url");
    assert!(err.is_err(), "invalid facilitator URL must return Err");
    match err {
        Err(FacilitatorClientError::UrlParse { context, .. }) => {
            assert_eq!(context, "Failed to parse base url");
        }
        other => panic!("expected UrlParse, got {other:?}"),
    }
}

#[tokio::test]
async fn resource_url_uses_base_url_not_host() {
    let fac = Arc::new(FakeFacilitator::new());
    let layer = X402Middleware::from_facilitator(Arc::clone(&fac))
        .with_base_url(base_url())
        .with_scheme(
            r402_protocol::network::ChainIdPattern::wildcard("eip155"),
            FlowScheme::authorization(),
        )
        .with_price_tag(eip155_tag())
        .unwrap();
    let req = axum_core::extract::Request::builder()
        .uri("/paid")
        .header("host", "evil.example")
        .body(axum_core::body::Body::empty())
        .unwrap();
    let response = call_layer(layer, OkInner, req).await;
    assert_eq!(response.status(), StatusCode::PAYMENT_REQUIRED);
    let header = response
        .headers()
        .get(r402_http::server::PAYMENT_REQUIRED)
        .unwrap();
    let decoded = r402_protocol::payment::Base64Bytes::from(header.as_bytes())
        .decode()
        .unwrap();
    let body: serde_json::Value = serde_json::from_slice(&decoded).unwrap();
    let url = body["resource"]["url"].as_str().unwrap();
    assert!(
        url.starts_with("https://api.example.com/"),
        "resource url must use configured base_url, got {url}"
    );
    assert!(
        !url.contains("evil.example"),
        "Host header must never be used, got {url}"
    );
    assert!(
        !url.contains("localhost"),
        "localhost must never be used, got {url}"
    );
}

#[tokio::test]
async fn with_resource_pins_url() {
    let fac = Arc::new(FakeFacilitator::new());
    let layer = X402Middleware::from_facilitator(Arc::clone(&fac))
        .with_base_url(base_url())
        .with_scheme(
            r402_protocol::network::ChainIdPattern::wildcard("eip155"),
            FlowScheme::authorization(),
        )
        .with_price_tag(eip155_tag())
        .unwrap()
        .with_resource("https://api.example.com/pinned".parse().unwrap());
    let response = call_layer(layer, OkInner, unpaid_request()).await;
    let header = response
        .headers()
        .get(r402_http::server::PAYMENT_REQUIRED)
        .unwrap();
    let decoded = r402_protocol::payment::Base64Bytes::from(header.as_bytes())
        .decode()
        .unwrap();
    let body: serde_json::Value = serde_json::from_slice(&decoded).unwrap();
    assert_eq!(body["resource"]["url"], "https://api.example.com/pinned");
}
