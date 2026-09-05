#![allow(unused_crate_dependencies, reason = "sibling tests consume them")]
#![allow(
    clippy::tests_outside_test_module,
    reason = "integration test binaries put #[test] fns at file scope"
)]
#![allow(
    clippy::doc_markdown,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::missing_assert_message,
    clippy::panic,
    clippy::unwrap_used,
    reason = "idiomatic test-code patterns"
)]

//! Construct-time `BuildError`. `with_resource` does not waive `base_url`.

mod harness;

use std::sync::Arc;

use alloy_primitives::address;
use http::StatusCode;
use r402_evm::{
    AssetTransferMethod, EIP2612_GAS_SPONSORING_KEY, ERC20_APPROVAL_GAS_SPONSORING_KEY,
    Eip155Exact, Eip155Upto, USDC,
};
use r402_extensions::{Eip2612GasSponsoringExtension, Erc20ApprovalGasSponsoringExtension};
use r402_http::server::{BuildError, FacilitatorClientError, SettlementMode, X402Middleware};
use r402_protocol::network::ChainIdPattern;
use r402_protocol::payment::{PaymentRequirements, PriceTag, SupportedPaymentKind};
use r402_server::PaymentFlowError;
use serde_json::json;

use crate::harness::{
    FakeFacilitator, FlowScheme, OkInner, base_url, call_layer, decode_payment_required,
    eip155_requirements, eip155_tag, escrow_tag, middleware, unpaid_request,
};

fn atm_tag() -> PriceTag {
    PriceTag::new(eip155_requirements_extra(json!({
        "assetTransferMethod": "not-an-atm"
    })))
}

fn unknown_flow_tag() -> PriceTag {
    PriceTag::new(eip155_requirements_extra(
        json!({ "paymentFlow": "not-a-flow" }),
    ))
}

fn upto_tag() -> PriceTag {
    PriceTag::new(PaymentRequirements::new(
        "upto".into(),
        "eip155:1".parse().unwrap(),
        "1000000".into(),
        "0xpay".into(),
        "0xasset".into(),
        60,
    ))
}

fn eip155_requirements_extra(extra: serde_json::Value) -> PaymentRequirements {
    eip155_requirements().with_extra(extra)
}

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
            ChainIdPattern::wildcard("eip155"),
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
            ChainIdPattern::wildcard("eip155"),
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

#[test]
fn with_price_tag_missing_scheme() {
    let err = X402Middleware::from_facilitator(FakeFacilitator::new())
        .with_base_url(base_url())
        .with_price_tag(eip155_tag())
        .expect_err("unregistered scheme must fail at construct");
    assert!(matches!(err, BuildError::MissingScheme { .. }));
}

#[test]
fn with_price_tag_unsupported_atm() {
    let err = middleware(
        Arc::new(FakeFacilitator::new()),
        FlowScheme::authorization(),
    )
    .with_price_tag(atm_tag())
    .expect_err("unsupported ATM must fail at construct");
    assert!(matches!(
        err,
        BuildError::PaymentFlow(PaymentFlowError::UnsupportedAssetTransferMethod { .. })
    ));
}

#[test]
fn with_price_tag_unknown_payment_flow() {
    let err = middleware(
        Arc::new(FakeFacilitator::new()),
        FlowScheme::authorization(),
    )
    .with_price_tag(unknown_flow_tag())
    .expect_err("unknown payment flow must fail at construct");
    assert!(matches!(
        err,
        BuildError::PaymentFlow(PaymentFlowError::UnsupportedPaymentFlow { .. })
    ));
}

#[test]
fn with_price_tags_empty_is_err() {
    let err = middleware(
        Arc::new(FakeFacilitator::new()),
        FlowScheme::authorization(),
    )
    .with_price_tags(vec![])
    .expect_err("empty static tags must fail without auth_only");
    assert_eq!(err, BuildError::EmptyPriceTags);
}

#[test]
fn with_price_tag_escrow_without_settles_on_cancel() {
    let err = middleware(
        Arc::new(FakeFacilitator::new()),
        FlowScheme::escrow_without_cancel(),
    )
    .with_price_tag(escrow_tag())
    .expect_err("escrow without cancel settle must fail at construct");
    assert!(matches!(err, BuildError::MissingSettleOnCancel { .. }));
}

#[test]
fn concurrent_escrow_is_mode_err() {
    let err = middleware(Arc::new(FakeFacilitator::new()), FlowScheme::escrow())
        .with_price_tag(escrow_tag())
        .unwrap()
        .with_settlement_mode(SettlementMode::Concurrent)
        .expect_err("Concurrent escrow must fail at construct");
    assert!(matches!(err, BuildError::Mode(_)));
}

#[test]
fn layer_with_price_tag_unregistered_scheme_is_err() {
    let layer = middleware(
        Arc::new(FakeFacilitator::new()),
        FlowScheme::authorization(),
    )
    .with_price_tag(eip155_tag())
    .unwrap();
    let err = layer
        .with_price_tag(upto_tag())
        .expect_err("appending an unregistered scheme must fail");
    assert!(matches!(err, BuildError::MissingScheme { .. }));
}

fn fac_kind(scheme: &str, network: &str) -> Arc<FakeFacilitator> {
    let fac = Arc::new(FakeFacilitator::new());
    *fac.supported_kinds.lock().unwrap() =
        Some(vec![SupportedPaymentKind::new(2, scheme, network)]);
    fac
}

const fn pay_to() -> alloy_primitives::Address {
    address!("0x1111111111111111111111111111111111111111")
}

fn usdc_upto_tag() -> PriceTag {
    Eip155Upto::price_tag(pay_to(), USDC::base().amount(1_000_000u64))
}

/// `upto` / `eip155:8453` with no extra — scheme default ATM is the Permit2 match.
fn upto_8453_no_extra_tag() -> PriceTag {
    PriceTag::new(PaymentRequirements::new(
        "upto".into(),
        "eip155:8453".parse().unwrap(),
        "1000000".into(),
        "0xpay".into(),
        "0xasset".into(),
        60,
    ))
}

fn usdc_exact_eip3009_tag() -> PriceTag {
    Eip155Exact::price_tag(pay_to(), USDC::base().amount(1_000_000u64), None)
}

fn usdc_exact_permit2_tag() -> PriceTag {
    Eip155Exact::price_tag(
        pay_to(),
        USDC::base().amount(1_000_000u64),
        Some(AssetTransferMethod::Permit2),
    )
}

fn assert_ext(body: &serde_json::Value, key: &str, present: bool) {
    let got = body.get("extensions").and_then(|ext| ext.get(key));
    if present {
        assert!(got.is_some(), "expected {key} on 402 extensions");
        let version = got
            .and_then(|v| v.get("info"))
            .and_then(|info| info.get("version"));
        assert_eq!(
            version,
            Some(&serde_json::Value::String("1".into())),
            "{key} advertise info.version must be 1"
        );
    } else {
        assert!(got.is_none(), "did not expect {key} on 402, got {got:?}");
    }
}

#[tokio::test]
async fn upto_402_has_2612_without_with_extension() {
    let fac = fac_kind("upto", "eip155:8453");
    let layer = X402Middleware::from_facilitator(Arc::clone(&fac))
        .with_base_url(base_url())
        .with_scheme(ChainIdPattern::wildcard("eip155"), Eip155Upto)
        .with_price_tag(usdc_upto_tag())
        .unwrap();
    let response = call_layer(layer, OkInner, unpaid_request()).await;
    assert_eq!(response.status(), StatusCode::PAYMENT_REQUIRED);
    let body = decode_payment_required(&response);
    assert_ext(&body, EIP2612_GAS_SPONSORING_KEY, true);
    assert_ext(&body, ERC20_APPROVAL_GAS_SPONSORING_KEY, false);
}

#[tokio::test]
async fn upto_402_without_extra_atm_matches_scheme_default_permit2() {
    let fac = fac_kind("upto", "eip155:8453");
    let layer = X402Middleware::from_facilitator(Arc::clone(&fac))
        .with_base_url(base_url())
        .with_scheme(ChainIdPattern::wildcard("eip155"), Eip155Upto)
        .with_price_tag(upto_8453_no_extra_tag())
        .unwrap();
    let response = call_layer(layer, OkInner, unpaid_request()).await;
    assert_eq!(
        response.status(),
        StatusCode::PAYMENT_REQUIRED,
        "unpaid upto with no extra must still 402"
    );
    let body = decode_payment_required(&response);
    let extra = body
        .get("accepts")
        .and_then(serde_json::Value::as_array)
        .and_then(|accepts| accepts.first())
        .and_then(|accept| accept.get("extra"));
    let atm = extra.and_then(|e| e.get("assetTransferMethod"));
    assert!(
        atm.is_none(),
        "fixture must omit extra ATM so the scheme default is the match, got {extra:?}"
    );
    assert_ext(&body, EIP2612_GAS_SPONSORING_KEY, true);
    assert_ext(&body, ERC20_APPROVAL_GAS_SPONSORING_KEY, false);
}

#[tokio::test]
async fn upto_402_has_erc20_only_when_supported_lists_it() {
    let fac = fac_kind("upto", "eip155:8453");
    *fac.supported_extensions.lock().unwrap() = vec![ERC20_APPROVAL_GAS_SPONSORING_KEY.into()];
    let layer = X402Middleware::from_facilitator(Arc::clone(&fac))
        .with_base_url(base_url())
        .with_scheme(ChainIdPattern::wildcard("eip155"), Eip155Upto)
        .with_price_tag(usdc_upto_tag())
        .unwrap();
    let response = call_layer(layer, OkInner, unpaid_request()).await;
    let body = decode_payment_required(&response);
    assert_ext(&body, EIP2612_GAS_SPONSORING_KEY, true);
    assert_ext(&body, ERC20_APPROVAL_GAS_SPONSORING_KEY, true);
}

#[tokio::test]
async fn eip3009_402_does_not_gain_gas_sponsoring_keys() {
    let fac = fac_kind("exact", "eip155:8453");
    *fac.supported_extensions.lock().unwrap() = vec![
        EIP2612_GAS_SPONSORING_KEY.into(),
        ERC20_APPROVAL_GAS_SPONSORING_KEY.into(),
    ];
    let layer = X402Middleware::from_facilitator(Arc::clone(&fac))
        .with_base_url(base_url())
        .with_scheme(ChainIdPattern::wildcard("eip155"), Eip155Exact)
        .with_price_tag(usdc_exact_eip3009_tag())
        .unwrap();
    let response = call_layer(layer, OkInner, unpaid_request()).await;
    let body = decode_payment_required(&response);
    assert_ext(&body, EIP2612_GAS_SPONSORING_KEY, false);
    assert_ext(&body, ERC20_APPROVAL_GAS_SPONSORING_KEY, false);
}

#[tokio::test]
async fn layer_with_extension_after_price_tag_advertises() {
    let fac = fac_kind("upto", "eip155:8453");
    let layer = X402Middleware::from_facilitator(Arc::clone(&fac))
        .with_base_url(base_url())
        .with_scheme(ChainIdPattern::wildcard("eip155"), Eip155Upto)
        .with_price_tag(usdc_upto_tag())
        .unwrap()
        .with_extension(Erc20ApprovalGasSponsoringExtension::new());
    let response = call_layer(layer, OkInner, unpaid_request()).await;
    let body = decode_payment_required(&response);
    assert_ext(&body, EIP2612_GAS_SPONSORING_KEY, true);
    assert_ext(&body, ERC20_APPROVAL_GAS_SPONSORING_KEY, true);
}

#[tokio::test]
async fn middleware_with_extension_advertises_erc20_when_supported_omits_it() {
    let fac = fac_kind("upto", "eip155:8453");
    let layer = X402Middleware::from_facilitator(Arc::clone(&fac))
        .with_extension(Erc20ApprovalGasSponsoringExtension::new())
        .with_base_url(base_url())
        .with_scheme(ChainIdPattern::wildcard("eip155"), Eip155Upto)
        .with_price_tag(usdc_upto_tag())
        .unwrap();
    let response = call_layer(layer, OkInner, unpaid_request()).await;
    let body = decode_payment_required(&response);
    assert_ext(&body, EIP2612_GAS_SPONSORING_KEY, true);
    assert_ext(&body, ERC20_APPROVAL_GAS_SPONSORING_KEY, true);
}

#[tokio::test]
async fn explicit_2612_is_not_duplicated() {
    let fac = fac_kind("upto", "eip155:8453");
    let layer = X402Middleware::from_facilitator(Arc::clone(&fac))
        .with_extension(Eip2612GasSponsoringExtension::new())
        .with_base_url(base_url())
        .with_scheme(ChainIdPattern::wildcard("eip155"), Eip155Upto)
        .with_price_tag(usdc_upto_tag())
        .unwrap();
    let response = call_layer(layer, OkInner, unpaid_request()).await;
    let body = decode_payment_required(&response);
    assert_ext(&body, EIP2612_GAS_SPONSORING_KEY, true);
    let extensions = body["extensions"].as_object().unwrap();
    let count = extensions
        .keys()
        .filter(|k| *k == EIP2612_GAS_SPONSORING_KEY)
        .count();
    assert_eq!(count, 1, "eip2612GasSponsoring must appear once on 402");
}

#[tokio::test]
async fn exact_permit2_dynamic_price_auto_inserts_2612() {
    let fac = fac_kind("exact", "eip155:8453");
    let tag = usdc_exact_permit2_tag();
    let layer = X402Middleware::from_facilitator(Arc::clone(&fac))
        .with_base_url(base_url())
        .with_scheme(ChainIdPattern::wildcard("eip155"), Eip155Exact)
        .with_dynamic_price(move |_, _, _| std::future::ready(vec![tag.clone()]))
        .unwrap();
    let response = call_layer(layer, OkInner, unpaid_request()).await;
    let body = decode_payment_required(&response);
    assert_ext(&body, EIP2612_GAS_SPONSORING_KEY, true);
    assert_ext(&body, ERC20_APPROVAL_GAS_SPONSORING_KEY, false);
}

#[tokio::test]
async fn exact_permit2_dynamic_price_erc20_iff_supported_lists_it() {
    let fac = fac_kind("exact", "eip155:8453");
    *fac.supported_extensions.lock().unwrap() = vec![ERC20_APPROVAL_GAS_SPONSORING_KEY.into()];
    let tag = usdc_exact_permit2_tag();
    let layer = X402Middleware::from_facilitator(Arc::clone(&fac))
        .with_base_url(base_url())
        .with_scheme(ChainIdPattern::wildcard("eip155"), Eip155Exact)
        .with_dynamic_price(move |_, _, _| std::future::ready(vec![tag.clone()]))
        .unwrap();
    let response = call_layer(layer, OkInner, unpaid_request()).await;
    let body = decode_payment_required(&response);
    assert_ext(&body, EIP2612_GAS_SPONSORING_KEY, true);
    assert_ext(&body, ERC20_APPROVAL_GAS_SPONSORING_KEY, true);
}
