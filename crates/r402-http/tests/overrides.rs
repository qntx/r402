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

//! Sequential actual amount from Settlement-Overrides / UptoActualAmount.

mod common;

use std::convert::Infallible;
use std::sync::Arc;
use std::task::{Context, Poll};

use axum_core::body::Body;
use axum_core::extract::Request;
use axum_core::response::Response;
use http::StatusCode;
use r402_http::server::{SettlementOverrides, UptoActualAmount, set_settlement_overrides};
use tower::Service;

use crate::common::{
    FakeFacilitator, FlowScheme, call_layer, eip155_requirements, eip155_tag, middleware,
    payment_request,
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

#[tokio::test]
async fn sequential_applies_percent_override() {
    let fac = Arc::new(FakeFacilitator::new());
    let layer = middleware(Arc::clone(&fac), FlowScheme::authorization())
        .with_price_tag(eip155_tag())
        .unwrap();
    let response = call_layer(layer, PercentInner, payment_request(&eip155_requirements())).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(fac.last_amount().as_deref(), Some("500000"));
    assert!(
        !response
            .headers()
            .contains_key(r402_http::server::settlement_overrides_header_name()),
        "billing header must be stripped"
    );
}

#[tokio::test]
async fn sequential_extension_beats_header() {
    let fac = Arc::new(FakeFacilitator::new());
    let layer = middleware(Arc::clone(&fac), FlowScheme::authorization())
        .with_price_tag(eip155_tag())
        .unwrap();
    let response = call_layer(layer, ExtInner, payment_request(&eip155_requirements())).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(fac.last_amount().as_deref(), Some("42"));
}
