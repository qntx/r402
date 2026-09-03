#![allow(unused_crate_dependencies, reason = "sibling tests consume them")]
#![allow(
    clippy::tests_outside_test_module,
    reason = "integration test binaries put #[test] fns at file scope"
)]
#![allow(
    clippy::doc_markdown,
    clippy::expect_used,
    clippy::panic,
    clippy::unwrap_used,
    reason = "idiomatic test-code patterns"
)]

//! Sequential upfront: one settle. Escrow: settle before + after.

mod harness;

use std::sync::Arc;

use http::StatusCode;
use r402_http::server::PAYMENT_RESPONSE;

use crate::harness::{
    FakeFacilitator, FlowScheme, OkInner, StatusInner, call_layer, escrow_tag, middleware,
    payment_request, unpaid_request, upfront_tag,
};

#[tokio::test]
async fn upfront_settles_once_before_handler() {
    let fac = Arc::new(FakeFacilitator::new());
    let layer = middleware(Arc::clone(&fac), FlowScheme::auth_and_upfront())
        .with_price_tag(upfront_tag())
        .unwrap();
    let accepted = &upfront_tag().requirements;
    let response = call_layer(layer, OkInner, payment_request(accepted)).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        fac.settle_count(),
        1,
        "upfront must settle once (before handler); finish(EchoReceipt)"
    );
    assert!(
        response.headers().get(PAYMENT_RESPONSE).is_some(),
        "upfront echoes the before-handler receipt"
    );
}

#[tokio::test]
async fn escrow_settles_twice() {
    let fac = Arc::new(FakeFacilitator::new());
    let layer = middleware(Arc::clone(&fac), FlowScheme::escrow())
        .with_price_tag(escrow_tag())
        .unwrap();
    let accepted = &escrow_tag().requirements;
    let response = call_layer(layer, OkInner, payment_request(accepted)).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        fac.settle_count(),
        2,
        "escrow settles before handler and finish(WaitThenSettle) after"
    );
    assert!(response.headers().get(PAYMENT_RESPONSE).is_some());
}

#[tokio::test]
async fn escrow_handler_4xx_runs_cancel_settle() {
    let fac = Arc::new(FakeFacilitator::new());
    let layer = middleware(Arc::clone(&fac), FlowScheme::escrow())
        .with_price_tag(escrow_tag())
        .unwrap();
    let response = call_layer(
        layer,
        StatusInner(StatusCode::BAD_REQUEST),
        payment_request(&escrow_tag().requirements),
    )
    .await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert!(
        response.headers().get(PAYMENT_RESPONSE).is_some(),
        "escrow 4xx must attach failure-path Payment-Response"
    );
    assert_eq!(
        fac.settle_count(),
        2,
        "escrow 4xx: before-handler deposit + cancel settle; never after-handler finish"
    );
    assert_eq!(fac.verify_count(), 0);
}

#[tokio::test]
async fn upfront_unpaid_is_402() {
    let fac = Arc::new(FakeFacilitator::new());
    let layer = middleware(Arc::clone(&fac), FlowScheme::auth_and_upfront())
        .with_price_tag(upfront_tag())
        .unwrap();
    let response = call_layer(layer, OkInner, unpaid_request()).await;
    assert_eq!(response.status(), StatusCode::PAYMENT_REQUIRED);
    assert_eq!(fac.settle_count(), 0);
}
