#![allow(unused_crate_dependencies, reason = "sibling tests consume them")]
#![allow(
    clippy::tests_outside_test_module,
    reason = "integration test binaries put #[test] fns at file scope"
)]
#![allow(
    clippy::doc_markdown,
    clippy::expect_used,
    clippy::manual_async_fn,
    clippy::panic,
    clippy::unwrap_used,
    reason = "idiomatic test-code patterns"
)]

//! Sequential happy path, unpaid 402, handler 4xx cancel, GrantAccess.

mod harness;

use std::future::Future;
use std::sync::Arc;

use http::StatusCode;
use r402_http::server::{
    GateHooks, PAYMENT_REQUIRED, PAYMENT_RESPONSE, ProtectedRequestOutcome, ResourceServer,
    SettlementMode,
};
use r402_http::{ensure_expose_headers, merge_private};
use r402_protocol::network::ChainIdPattern;

use crate::harness::{
    FakeFacilitator, FlowScheme, OkInner, SkipAfterVerify, SkipVerifyThenHandler, StatusInner,
    call_layer, eip155_requirements, eip155_tag, middleware, middleware_on, payment_request,
    unpaid_request, upfront_tag,
};

struct GrantAll;

impl GateHooks for GrantAll {
    fn on_protected_request<'a>(
        &'a self,
        _req: &'a axum_core::extract::Request,
    ) -> impl Future<Output = ProtectedRequestOutcome> + Send + 'a {
        async { ProtectedRequestOutcome::GrantAccess }
    }
}

#[tokio::test]
async fn unpaid_is_402_with_payment_required() {
    let fac = Arc::new(FakeFacilitator::new());
    let layer = middleware(Arc::clone(&fac), FlowScheme::authorization())
        .with_price_tag(eip155_tag())
        .unwrap()
        .with_settlement_mode(SettlementMode::Sequential);
    let response = call_layer(layer, OkInner, unpaid_request()).await;
    assert_eq!(response.status(), StatusCode::PAYMENT_REQUIRED);
    assert!(
        response.headers().get(PAYMENT_REQUIRED).is_some(),
        "402 must carry Payment-Required"
    );
    assert_eq!(fac.settle_count(), 0, "unpaid must not settle");
}

#[tokio::test]
async fn paid_sequential_attaches_payment_response() {
    let fac = Arc::new(FakeFacilitator::new());
    let layer = middleware(Arc::clone(&fac), FlowScheme::authorization())
        .with_price_tag(eip155_tag())
        .unwrap();
    let response = call_layer(layer, OkInner, payment_request(&eip155_requirements())).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert!(
        response.headers().get(PAYMENT_RESPONSE).is_some(),
        "success must attach Payment-Response"
    );
    assert_eq!(
        fac.settle_count(),
        1,
        "authorization settles once after handler"
    );
    let cc = response
        .headers()
        .get(http::header::CACHE_CONTROL)
        .and_then(|v| v.to_str().ok());
    assert_eq!(cc, Some("private"), "200 + receipt merges private");
}

#[tokio::test]
async fn handler_4xx_skips_after_handler_settle() {
    let fac = Arc::new(FakeFacilitator::new());
    let layer = middleware(Arc::clone(&fac), FlowScheme::authorization())
        .with_price_tag(eip155_tag())
        .unwrap();
    let response = call_layer(
        layer,
        StatusInner(StatusCode::BAD_REQUEST),
        payment_request(&eip155_requirements()),
    )
    .await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert_eq!(
        fac.settle_count(),
        0,
        "handler 4xx must not start after-handler settle"
    );
    assert!(
        response.headers().get(PAYMENT_RESPONSE).is_none(),
        "authorization 4xx has no before-handler receipt to echo"
    );
}

#[tokio::test]
async fn skip_handler_authorization_settles_once() {
    let fac = Arc::new(FakeFacilitator::new());
    let mut server = ResourceServer::new(Arc::clone(&fac));
    server.register_scheme(
        ChainIdPattern::wildcard("eip155"),
        FlowScheme::authorization(),
    );
    server.add_hook(SkipAfterVerify);
    let layer = middleware_on(server).with_price_tag(eip155_tag()).unwrap();
    let response = call_layer(
        layer,
        StatusInner(StatusCode::BAD_REQUEST),
        payment_request(&eip155_requirements()),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(fac.verify_count(), 1);
    assert_eq!(
        fac.settle_count(),
        1,
        "skip_handler authorization settles once"
    );
    assert!(response.headers().get(PAYMENT_RESPONSE).is_some());
}

#[tokio::test]
async fn skip_handler_upfront_does_not_settle() {
    let fac = Arc::new(FakeFacilitator::new());
    let mut server = ResourceServer::new(Arc::clone(&fac));
    server.register_scheme(
        ChainIdPattern::wildcard("eip155"),
        FlowScheme::auth_and_upfront(),
    );
    server.add_hook(SkipVerifyThenHandler);
    let layer = middleware_on(server).with_price_tag(upfront_tag()).unwrap();
    let response = call_layer(
        layer,
        StatusInner(StatusCode::BAD_REQUEST),
        payment_request(&upfront_tag().requirements),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(fac.verify_count(), 0);
    assert_eq!(
        fac.settle_count(),
        0,
        "skip_handler upfront must not settle"
    );
}

#[tokio::test]
async fn grant_access_bypasses_payment() {
    let fac = Arc::new(FakeFacilitator::new());
    let layer = middleware(Arc::clone(&fac), FlowScheme::authorization())
        .with_price_tag(eip155_tag())
        .unwrap()
        .with_hooks(GrantAll);
    let response = call_layer(layer, OkInner, unpaid_request()).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(fac.settle_count(), 0);
    assert!(response.headers().get(PAYMENT_REQUIRED).is_none());
}

#[tokio::test]
async fn empty_tags_bypass() {
    let fac = Arc::new(FakeFacilitator::new());
    let layer = middleware(Arc::clone(&fac), FlowScheme::authorization())
        .with_price_tags(vec![])
        .unwrap();
    let response = call_layer(layer, OkInner, unpaid_request()).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(fac.settle_count(), 0);
}

#[test]
fn cors_helper_is_available() {
    let mut headers = http::HeaderMap::new();
    ensure_expose_headers(&mut headers);
    merge_private(&mut headers);
    assert!(headers.contains_key(http::header::ACCESS_CONTROL_EXPOSE_HEADERS));
    assert!(headers.contains_key(http::header::CACHE_CONTROL));
}
