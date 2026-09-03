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

//! Concurrent: JoinSettle, overrides → 402, 4xx detaches settle.

mod harness;

use std::convert::Infallible;
use std::future::Future;
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::Duration;

use axum_core::body::Body;
use axum_core::extract::Request;
use axum_core::response::Response;
use http::StatusCode;
use r402_http::server::{
    PAYMENT_RESPONSE, SettlementMode, SettlementOverrides, UptoActualAmount,
    set_settlement_overrides, settlement_overrides_header_name,
};
use tokio::sync::Notify;
use tower::Service;

use crate::harness::{
    FakeFacilitator, FlowScheme, OkInner, SettleScript, StatusInner, call_layer,
    eip155_requirements, eip155_tag, escrow_tag, middleware, payment_request, unpaid_request,
};

#[derive(Clone)]
struct WaitInner(Arc<Notify>);

impl Service<Request> for WaitInner {
    type Response = Response;
    type Error = Infallible;
    type Future = std::pin::Pin<Box<dyn Future<Output = Result<Response, Infallible>> + Send>>;

    fn poll_ready(&mut self, _: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, _: Request) -> Self::Future {
        let notify = Arc::clone(&self.0);
        Box::pin(async move {
            notify.notified().await;
            Ok(Response::new(Body::from("ok")))
        })
    }
}

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
        response
            .extensions_mut()
            .insert(UptoActualAmount::new("42"));
        std::future::ready(Ok(response))
    }
}

fn concurrent_layer(
    fac: Arc<FakeFacilitator>,
) -> r402_http::server::X402Layer<r402_http::server::StaticPriceTags> {
    middleware(fac, FlowScheme::authorization())
        .with_price_tag(eip155_tag())
        .unwrap()
        .with_settlement_mode(SettlementMode::Concurrent)
}

#[tokio::test]
async fn paid_concurrent_attaches_payment_response() {
    let fac = Arc::new(FakeFacilitator::new());
    let response = call_layer(
        concurrent_layer(Arc::clone(&fac)),
        OkInner,
        payment_request(&eip155_requirements()),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    assert!(
        response.headers().get(PAYMENT_RESPONSE).is_some(),
        "Concurrent success must attach Payment-Response"
    );
    assert_eq!(fac.settle_count(), 1);
    assert_eq!(fac.verify_count(), 1);
}

#[tokio::test]
async fn settle_starts_while_handler_waits() {
    let fac = Arc::new(FakeFacilitator::new());
    let notify = Arc::new(Notify::new());
    let handle = tokio::spawn(call_layer(
        concurrent_layer(Arc::clone(&fac)),
        WaitInner(Arc::clone(&notify)),
        payment_request(&eip155_requirements()),
    ));
    tokio::time::timeout(Duration::from_secs(1), async {
        while fac.settle_count() == 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("Concurrent must start settle before the handler returns");
    notify.notify_one();
    let response = handle.await.expect("join");
    assert_eq!(response.status(), StatusCode::OK);
    assert!(response.headers().get(PAYMENT_RESPONSE).is_some());
}

#[tokio::test]
async fn handler_4xx_detaches_settle() {
    let fac = Arc::new(FakeFacilitator::new());
    let hold = fac.hold_settle();
    let response = tokio::time::timeout(
        Duration::from_millis(200),
        call_layer(
            concurrent_layer(Arc::clone(&fac)),
            StatusInner(StatusCode::BAD_REQUEST),
            payment_request(&eip155_requirements()),
        ),
    )
    .await
    .expect("4xx must drop settle and return without joining");
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert!(
        response.headers().get(PAYMENT_RESPONSE).is_none(),
        "authorization Concurrent 4xx has no receipt to attach"
    );
    assert!(
        !hold.is_closed(),
        "dropping the join handle detaches settle; it must not abort"
    );
    hold.send(()).expect("release detached settle");
    tokio::time::timeout(Duration::from_secs(1), async {
        while fac.settle_count() == 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("detached settle must still complete");
}

#[tokio::test]
async fn overrides_are_402() {
    let fac = Arc::new(FakeFacilitator::new());
    let response = call_layer(
        concurrent_layer(Arc::clone(&fac)),
        PercentInner,
        payment_request(&eip155_requirements()),
    )
    .await;
    assert_eq!(response.status(), StatusCode::PAYMENT_REQUIRED);
    assert!(
        !response
            .headers()
            .contains_key(settlement_overrides_header_name()),
        "billing header must not reach the client"
    );
}

#[tokio::test]
async fn upto_actual_amount_is_402() {
    let fac = Arc::new(FakeFacilitator::new());
    let response = call_layer(
        concurrent_layer(Arc::clone(&fac)),
        ExtInner,
        payment_request(&eip155_requirements()),
    )
    .await;
    assert_eq!(response.status(), StatusCode::PAYMENT_REQUIRED);
}

#[tokio::test]
async fn settle_failure_is_402() {
    let fac = Arc::new(FakeFacilitator::new());
    *fac.settle.lock().unwrap() = SettleScript::Fail;
    let response = call_layer(
        concurrent_layer(Arc::clone(&fac)),
        OkInner,
        payment_request(&eip155_requirements()),
    )
    .await;
    assert_eq!(response.status(), StatusCode::PAYMENT_REQUIRED);
}

#[tokio::test]
async fn mixed_auth_and_escrow_is_500() {
    let fac = Arc::new(FakeFacilitator::new());
    let layer = middleware(Arc::clone(&fac), FlowScheme::auth_and_escrow())
        .with_price_tags(vec![eip155_tag(), escrow_tag()])
        .unwrap()
        .with_settlement_mode(SettlementMode::Concurrent);
    let response = call_layer(layer, OkInner, unpaid_request()).await;
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(fac.settle_count(), 0);
}
