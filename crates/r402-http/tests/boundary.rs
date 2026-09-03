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

//! 502 vs 402 vs 500 vs 412.

mod harness;

use std::collections::HashMap;
use std::sync::Arc;

use compact_str::CompactString;
use http::{HeaderValue, StatusCode};
use r402_http::server::{
    BuildError, PAYMENT_REQUIRED, PAYMENT_SIGNATURE, SettlementMode, X402Middleware,
};
use r402_protocol::error::ErrorReason;
use r402_protocol::network::{ChainId, ChainIdPattern};
use r402_protocol::payment::SupportedPaymentKind;
use r402_server::{FacilitatorSupportError, PaymentFlowConfig, SchemeNetworkServer};

use crate::harness::{
    FakeFacilitator, FlowScheme, OkInner, SettleScript, SupportedScript, VerifyScript, call_layer,
    eip155_requirements, eip155_tag, escrow_tag, json_body, middleware, payment_request,
    unpaid_request, upfront_tag,
};

struct FeePayerScheme(FlowScheme);

impl SchemeNetworkServer for FeePayerScheme {
    fn scheme(&self) -> &str {
        self.0.scheme()
    }

    fn default_asset_transfer_method(&self) -> &str {
        self.0.default_asset_transfer_method()
    }

    fn payment_flows(&self) -> &HashMap<String, PaymentFlowConfig> {
        self.0.payment_flows()
    }

    fn validate_facilitator_support(
        &self,
        network: &ChainId,
        kind: &SupportedPaymentKind,
        _facilitator_extensions: &[CompactString],
    ) -> Result<(), FacilitatorSupportError> {
        let has_fee_payer = kind
            .extra
            .as_ref()
            .and_then(|extra| extra.get("feePayer"))
            .and_then(serde_json::Value::as_str)
            .is_some();
        if has_fee_payer {
            Ok(())
        } else {
            Err(FacilitatorSupportError::MissingFeePayer {
                scheme: self.scheme().into(),
                network: network.clone(),
            })
        }
    }
}

#[tokio::test]
async fn transport_is_502() {
    let fac = Arc::new(FakeFacilitator::new());
    *fac.verify.lock().unwrap() = VerifyScript::Transport;
    let layer = middleware(Arc::clone(&fac), FlowScheme::authorization())
        .with_price_tag(eip155_tag())
        .unwrap();
    let response = call_layer(layer, OkInner, payment_request(&eip155_requirements())).await;
    assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
}

#[tokio::test]
async fn settle_transport_is_502() {
    let fac = Arc::new(FakeFacilitator::new());
    *fac.settle.lock().unwrap() = SettleScript::Transport;
    let layer = middleware(Arc::clone(&fac), FlowScheme::authorization())
        .with_price_tag(eip155_tag())
        .unwrap();
    let response = call_layer(layer, OkInner, payment_request(&eip155_requirements())).await;
    assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
}

#[tokio::test]
async fn malformed_payment_signature_is_402() {
    let fac = Arc::new(FakeFacilitator::new());
    let layer = middleware(Arc::clone(&fac), FlowScheme::authorization())
        .with_price_tag(eip155_tag())
        .unwrap();
    let req = axum_core::extract::Request::builder()
        .uri("/paid")
        .header(
            PAYMENT_SIGNATURE,
            HeaderValue::from_static("not-valid-base64!!!"),
        )
        .body(axum_core::body::Body::empty())
        .unwrap();
    let response = call_layer(layer, OkInner, req).await;
    assert_eq!(response.status(), StatusCode::PAYMENT_REQUIRED);
}

#[tokio::test]
async fn invalid_verify_is_402() {
    let fac = Arc::new(FakeFacilitator::new());
    *fac.verify.lock().unwrap() = VerifyScript::Invalid(ErrorReason::InsufficientFunds);
    let layer = middleware(Arc::clone(&fac), FlowScheme::authorization())
        .with_price_tag(eip155_tag())
        .unwrap();
    let response = call_layer(layer, OkInner, payment_request(&eip155_requirements())).await;
    assert_eq!(response.status(), StatusCode::PAYMENT_REQUIRED);
}

#[tokio::test]
async fn permit2_is_412() {
    let fac = Arc::new(FakeFacilitator::new());
    *fac.verify.lock().unwrap() = VerifyScript::Invalid(ErrorReason::Permit2AllowanceRequired);
    let layer = middleware(Arc::clone(&fac), FlowScheme::authorization())
        .with_price_tag(eip155_tag())
        .unwrap();
    let response = call_layer(layer, OkInner, payment_request(&eip155_requirements())).await;
    assert_eq!(response.status(), StatusCode::PRECONDITION_FAILED);
}

#[test]
fn missing_scheme_is_build_error() {
    let fac = Arc::new(FakeFacilitator::new());
    let err = X402Middleware::from_facilitator(Arc::clone(&fac))
        .with_base_url(harness::base_url())
        .with_price_tag(eip155_tag())
        .expect_err("unregistered scheme must fail at construct");
    assert!(matches!(err, BuildError::MissingScheme { .. }));
}

#[test]
fn concurrent_upfront_is_build_error() {
    let fac = Arc::new(FakeFacilitator::new());
    let err = middleware(Arc::clone(&fac), FlowScheme::auth_and_upfront())
        .with_price_tag(upfront_tag())
        .unwrap()
        .with_settlement_mode(SettlementMode::Concurrent)
        .expect_err("Concurrent upfront must fail at construct");
    assert!(matches!(err, BuildError::Mode(_)));
}

#[test]
fn background_upfront_is_build_error() {
    let fac = Arc::new(FakeFacilitator::new());
    let err = middleware(Arc::clone(&fac), FlowScheme::auth_and_upfront())
        .with_scheme(
            ChainIdPattern::wildcard("eip155"),
            FlowScheme::auth_and_upfront(),
        )
        .with_price_tag(upfront_tag())
        .unwrap()
        .with_settlement_mode(SettlementMode::Background)
        .expect_err("Background upfront must fail at construct");
    assert!(matches!(err, BuildError::Mode(_)));
}

#[tokio::test]
async fn supported_empty_kinds_is_500_without_payment_required() {
    let fac = Arc::new(FakeFacilitator::new());
    *fac.supported.lock().unwrap() = SupportedScript::Empty;
    let layer = middleware(Arc::clone(&fac), FlowScheme::authorization())
        .with_price_tag(eip155_tag())
        .unwrap();
    let response = call_layer(layer, OkInner, unpaid_request()).await;
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    assert!(response.headers().get(PAYMENT_REQUIRED).is_none());
    let body = json_body(response).await;
    assert_eq!(body["error"], "facilitator support");
    assert_eq!(body["reason"], "kind_missing");
    assert_eq!(body["scheme"], "exact");
    assert_eq!(body["network"], "eip155:1");
}

#[tokio::test]
async fn supported_kind_missing_for_accept_is_500() {
    let fac = Arc::new(FakeFacilitator::new());
    *fac.supported.lock().unwrap() = SupportedScript::Mismatch;
    let layer = middleware(Arc::clone(&fac), FlowScheme::authorization())
        .with_price_tag(eip155_tag())
        .unwrap();
    let response = call_layer(layer, OkInner, unpaid_request()).await;
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    assert!(response.headers().get(PAYMENT_REQUIRED).is_none());
    let body = json_body(response).await;
    assert_eq!(body["error"], "facilitator support");
    assert_eq!(body["reason"], "kind_missing");
    assert_eq!(body["scheme"], "exact");
    assert_eq!(body["network"], "eip155:1");
}

#[tokio::test]
async fn supported_missing_fee_payer_is_500() {
    let fac = Arc::new(FakeFacilitator::new());
    let layer = X402Middleware::from_facilitator(Arc::clone(&fac))
        .with_base_url(harness::base_url())
        .with_scheme(
            ChainIdPattern::wildcard("eip155"),
            FeePayerScheme(FlowScheme::authorization()),
        )
        .with_price_tag(eip155_tag())
        .unwrap();
    let response = call_layer(layer, OkInner, unpaid_request()).await;
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    assert!(response.headers().get(PAYMENT_REQUIRED).is_none());
    let body = json_body(response).await;
    assert_eq!(body["error"], "facilitator support");
    assert_eq!(body["reason"], "missing_fee_payer");
    assert_eq!(body["scheme"], "exact");
    assert_eq!(body["network"], "eip155:1");
}

#[tokio::test]
async fn supported_transport_is_502_without_payment_required() {
    let fac = Arc::new(FakeFacilitator::new());
    *fac.supported.lock().unwrap() = SupportedScript::Transport;
    let layer = middleware(Arc::clone(&fac), FlowScheme::authorization())
        .with_price_tag(eip155_tag())
        .unwrap();
    let response = call_layer(layer, OkInner, unpaid_request()).await;
    assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
    assert!(response.headers().get(PAYMENT_REQUIRED).is_none());
}

#[tokio::test]
async fn settle_failure_is_402() {
    let fac = Arc::new(FakeFacilitator::new());
    *fac.settle.lock().unwrap() = SettleScript::Fail;
    let layer = middleware(Arc::clone(&fac), FlowScheme::authorization())
        .with_price_tag(eip155_tag())
        .unwrap();
    let response = call_layer(layer, OkInner, payment_request(&eip155_requirements())).await;
    assert_eq!(response.status(), StatusCode::PAYMENT_REQUIRED);
}

#[tokio::test]
async fn dynamic_missing_scheme_is_500() {
    let fac = Arc::new(FakeFacilitator::new());
    let layer = X402Middleware::from_facilitator(Arc::clone(&fac))
        .with_base_url(harness::base_url())
        .with_dynamic_price(|_, _, _| std::future::ready(vec![eip155_tag()]))
        .unwrap();
    let response = call_layer(layer, OkInner, unpaid_request()).await;
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    assert!(response.headers().get(PAYMENT_REQUIRED).is_none());
    let body = json_body(response).await;
    assert_eq!(body["error"], "missing scheme");
    assert_eq!(body["scheme"], "exact");
    assert_eq!(body["network"], "eip155:1");
}

#[tokio::test]
async fn dynamic_concurrent_upfront_is_500() {
    let fac = Arc::new(FakeFacilitator::new());
    let layer = middleware(Arc::clone(&fac), FlowScheme::auth_and_upfront())
        .with_dynamic_price(|_, _, _| std::future::ready(vec![upfront_tag()]))
        .unwrap()
        .with_settlement_mode(SettlementMode::Concurrent)
        .expect("dynamic mode is stored without static validation");
    let response = call_layer(layer, OkInner, unpaid_request()).await;
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    assert!(response.headers().get(PAYMENT_REQUIRED).is_none());
    let body = json_body(response).await;
    assert_eq!(body["error"], "incompatible settlement mode");
    assert_eq!(body["mode"], "concurrent");
    assert_eq!(body["flow"], "upfront");
}

#[tokio::test]
async fn dynamic_background_upfront_is_500() {
    let fac = Arc::new(FakeFacilitator::new());
    let layer = middleware(Arc::clone(&fac), FlowScheme::auth_and_upfront())
        .with_dynamic_price(|_, _, _| std::future::ready(vec![upfront_tag()]))
        .unwrap()
        .with_settlement_mode(SettlementMode::Background)
        .expect("dynamic mode is stored without static validation");
    let response = call_layer(layer, OkInner, unpaid_request()).await;
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    assert!(response.headers().get(PAYMENT_REQUIRED).is_none());
    let body = json_body(response).await;
    assert_eq!(body["error"], "incompatible settlement mode");
    assert_eq!(body["mode"], "background");
    assert_eq!(body["flow"], "upfront");
}

#[tokio::test]
async fn dynamic_concurrent_escrow_is_500() {
    let fac = Arc::new(FakeFacilitator::new());
    let layer = middleware(Arc::clone(&fac), FlowScheme::auth_and_escrow())
        .with_dynamic_price(|_, _, _| std::future::ready(vec![eip155_tag(), escrow_tag()]))
        .unwrap()
        .with_settlement_mode(SettlementMode::Concurrent)
        .expect("dynamic mode is stored without static validation");
    let response = call_layer(layer, OkInner, unpaid_request()).await;
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    assert!(response.headers().get(PAYMENT_REQUIRED).is_none());
    let body = json_body(response).await;
    assert_eq!(body["error"], "incompatible settlement mode");
    assert_eq!(body["mode"], "concurrent");
    assert_eq!(body["flow"], "escrow");
}

#[tokio::test]
async fn dynamic_escrow_without_cancel_is_500() {
    let fac = Arc::new(FakeFacilitator::new());
    let layer = middleware(Arc::clone(&fac), FlowScheme::escrow_without_cancel())
        .with_dynamic_price(|_, _, _| std::future::ready(vec![escrow_tag()]))
        .unwrap();
    let response = call_layer(layer, OkInner, unpaid_request()).await;
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    assert!(response.headers().get(PAYMENT_REQUIRED).is_none());
    let body = json_body(response).await;
    assert_eq!(body["error"], "missing settle_on_cancel");
    assert_eq!(body["scheme"], "exact");
}
