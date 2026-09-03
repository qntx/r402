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

//! Background: SpawnSettle, strip overrides, no Payment-Response.

mod harness;

use std::convert::Infallible;
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::Duration;

use axum_core::body::Body;
use axum_core::extract::Request;
use axum_core::response::Response;
use http::StatusCode;
use http::header::CACHE_CONTROL;
use r402_http::server::{
    BackgroundSettlementTracker, PAYMENT_RESPONSE, SettlementMode, SettlementOverrides,
    UptoActualAmount, set_settlement_overrides, settlement_overrides_header_name,
};
use tower::Service;

use crate::harness::{
    FakeFacilitator, FlowScheme, OkInner, SettleScript, StatusInner, call_layer,
    eip155_requirements, eip155_tag, middleware, payment_request,
};

#[derive(Clone)]
struct PercentInner;

impl Service<Request> for PercentInner {
    type Response = Response;
    type Error = Infallible;
    type Future = std::future::Ready<Result<Response, Infallible>>;

    fn poll_ready(&mut self, _: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, _: Request) -> Self::Future {
        let mut response = Response::new(Body::from("ok"));
        set_settlement_overrides(response.headers_mut(), &SettlementOverrides::amount("50%"));
        std::future::ready(Ok(response))
    }
}

#[derive(Clone)]
struct ExtInner;

impl Service<Request> for ExtInner {
    type Response = Response;
    type Error = Infallible;
    type Future = std::future::Ready<Result<Response, Infallible>>;

    fn poll_ready(&mut self, _: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, _: Request) -> Self::Future {
        let mut response = Response::new(Body::from("ok"));
        set_settlement_overrides(response.headers_mut(), &SettlementOverrides::amount("50%"));
        response
            .extensions_mut()
            .insert(UptoActualAmount::new("42"));
        std::future::ready(Ok(response))
    }
}

fn background_layer(
    fac: Arc<FakeFacilitator>,
    tracker: BackgroundSettlementTracker,
) -> r402_http::server::X402Layer<r402_http::server::StaticPriceTags> {
    middleware(fac, FlowScheme::authorization())
        .with_price_tag(eip155_tag())
        .unwrap()
        .with_settlement_mode(SettlementMode::Background)
        .with_settlement_tracker(tracker)
}

#[tokio::test]
async fn paid_background_omits_payment_response() {
    let fac = Arc::new(FakeFacilitator::new());
    let tracker = BackgroundSettlementTracker::new();
    let response = call_layer(
        background_layer(Arc::clone(&fac), tracker.clone()),
        OkInner,
        payment_request(&eip155_requirements()),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    assert!(
        response.headers().get(PAYMENT_RESPONSE).is_none(),
        "Background must not attach Payment-Response"
    );
    assert!(
        response.headers().get(CACHE_CONTROL).is_none(),
        "no receipt must not force Cache-Control: private"
    );
    tracker
        .wait_for_drain(Duration::from_secs(1))
        .await
        .expect("background settle drains");
    assert_eq!(fac.verify_count(), 1);
    assert_eq!(fac.settle_count(), 1);
}

#[tokio::test]
async fn returns_before_settle_completes() {
    let fac = Arc::new(FakeFacilitator::new());
    let hold = fac.hold_settle();
    let tracker = BackgroundSettlementTracker::new();
    let response = tokio::time::timeout(
        Duration::from_millis(200),
        call_layer(
            background_layer(Arc::clone(&fac), tracker.clone()),
            OkInner,
            payment_request(&eip155_requirements()),
        ),
    )
    .await
    .expect("Background must return without awaiting settle");
    assert_eq!(response.status(), StatusCode::OK);
    assert!(response.headers().get(PAYMENT_RESPONSE).is_none());
    assert!(
        !hold.is_closed(),
        "Background must detach settle; returning must not abort the in-flight task"
    );
    hold.send(()).expect("release background settle");
    tracker
        .wait_for_drain(Duration::from_secs(1))
        .await
        .expect("background settle drains");
    assert_eq!(fac.settle_count(), 1);
}

#[tokio::test]
async fn strips_overrides_without_402() {
    let fac = Arc::new(FakeFacilitator::new());
    let tracker = BackgroundSettlementTracker::new();
    let response = call_layer(
        background_layer(Arc::clone(&fac), tracker.clone()),
        PercentInner,
        payment_request(&eip155_requirements()),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    assert!(
        !response
            .headers()
            .contains_key(settlement_overrides_header_name()),
        "Settlement-Overrides must be stripped"
    );
    assert!(response.headers().get(PAYMENT_RESPONSE).is_none());
    tracker
        .wait_for_drain(Duration::from_secs(1))
        .await
        .expect("background settle drains");
    assert_eq!(
        fac.last_amount().as_deref(),
        Some("1000000"),
        "Background settle uses the signed max, not the stripped override"
    );
}

#[tokio::test]
async fn strips_upto_actual_amount_without_402() {
    let fac = Arc::new(FakeFacilitator::new());
    let tracker = BackgroundSettlementTracker::new();
    let response = call_layer(
        background_layer(Arc::clone(&fac), tracker.clone()),
        ExtInner,
        payment_request(&eip155_requirements()),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    assert!(
        !response
            .headers()
            .contains_key(settlement_overrides_header_name()),
        "Settlement-Overrides must be stripped even when the extension is present"
    );
    tracker
        .wait_for_drain(Duration::from_secs(1))
        .await
        .expect("background settle drains");
    assert_eq!(fac.last_amount().as_deref(), Some("1000000"));
}

#[tokio::test]
async fn handler_4xx_still_spawns_settle() {
    let fac = Arc::new(FakeFacilitator::new());
    let tracker = BackgroundSettlementTracker::new();
    let response = call_layer(
        background_layer(Arc::clone(&fac), tracker.clone()),
        StatusInner(StatusCode::BAD_REQUEST),
        payment_request(&eip155_requirements()),
    )
    .await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert!(response.headers().get(PAYMENT_RESPONSE).is_none());
    tracker
        .wait_for_drain(Duration::from_secs(1))
        .await
        .expect("background settle drains");
    assert_eq!(fac.settle_count(), 1);
}

#[tokio::test]
async fn settle_failure_is_non_fatal() {
    let fac = Arc::new(FakeFacilitator::new());
    *fac.settle.lock().unwrap() = SettleScript::Fail;
    let tracker = BackgroundSettlementTracker::new();
    let response = call_layer(
        background_layer(Arc::clone(&fac), tracker.clone()),
        OkInner,
        payment_request(&eip155_requirements()),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    assert!(response.headers().get(PAYMENT_RESPONSE).is_none());
    tracker
        .wait_for_drain(Duration::from_secs(1))
        .await
        .expect("failed background settle still drains");
}
