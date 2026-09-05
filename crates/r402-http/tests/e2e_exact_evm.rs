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

//! HTTP × EVM exact Sequential E2E.
//!
//! Default CI path records facilitator `/supported`, `/verify`, `/settle` with
//! wiremock so the suite is deterministic. `X402Middleware` Sequential
//! `with_price_tag` plus `X402Client` 402 retry is the happiness path.
//!
//! Set `R402_ANVIL=1` to also run the in-process [`Eip155ExactFacilitator`]
//! path against Anvil at `http://127.0.0.1:8545`.

use std::collections::HashMap;
use std::str::FromStr;

use alloy_network::EthereumWallet;
use alloy_primitives::Address;
use alloy_provider::Provider;
use alloy_signer_local::PrivateKeySigner;
use axum::Router;
use axum::routing::get;
use http::StatusCode;
use r402_evm::chain::{Eip155ChainProvider, Eip155ChainReference, Eip155MetaTransactionProvider};
use r402_evm::{
    AssetTransferMethod, EIP2612_GAS_SPONSORING_KEY, ERC20_APPROVAL_GAS_SPONSORING_KEY,
    Eip155Exact, Eip155ExactClient, Eip155ExactFacilitator, Eip155Upto, USDC,
};
use r402_extensions::Erc20ApprovalGasSponsoringExtension;
use r402_http::server::{PAYMENT_REQUIRED, PAYMENT_RESPONSE, PAYMENT_SIGNATURE, SettlementMode};
use r402_http::{WithPayments, X402Client, X402Middleware};
use r402_protocol::error::ErrorReason;
use r402_protocol::network::ChainIdPattern;
use r402_protocol::payment::{
    Base64Bytes, Extensions, SettleResponse, SupportedPaymentKind, SupportedResponse,
    VerifyResponse,
};
use tokio::net::TcpListener;
use url::Url;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// Anvil account 0. Local EIP-712 signing; never broadcast in the wiremock path.
const ANVIL_KEY: &str = "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80";

const ANVIL_RPC: &str = "http://127.0.0.1:8545";

const fn pay_to() -> Address {
    alloy_primitives::address!("0x1111111111111111111111111111111111111111")
}

fn anvil_opted_in() -> bool {
    matches!(std::env::var("R402_ANVIL"), Ok(value) if value == "1")
}

fn eip3009_tag() -> r402_protocol::payment::PriceTag {
    Eip155Exact::price_tag(pay_to(), USDC::base().amount(1_000_000u64), None)
}

fn permit2_tag() -> r402_protocol::payment::PriceTag {
    Eip155Exact::price_tag(
        pay_to(),
        USDC::base().amount(1_000_000u64),
        Some(AssetTransferMethod::Permit2),
    )
}

fn paid_buyer() -> X402Client {
    let signer = PrivateKeySigner::from_str(ANVIL_KEY).expect("anvil key");
    X402Client::new()
        .disable_spend_controls()
        .register(Eip155ExactClient::new(signer))
}

fn http_client() -> reqwest::Client {
    reqwest::Client::builder().no_proxy().build().unwrap()
}

fn supported_body() -> SupportedResponse {
    supported_body_for("exact", &[])
}

fn supported_body_for(scheme: &str, extensions: &[&str]) -> SupportedResponse {
    let mut signers = HashMap::new();
    signers.insert("eip155:8453".into(), vec!["0xfeePayer".into()]);
    SupportedResponse::new()
        .with_kinds(vec![SupportedPaymentKind::new(2, scheme, "eip155:8453")])
        .with_signers(signers)
        .with_extensions(extensions.iter().map(|e| (*e).into()).collect())
}

fn settle_ok() -> SettleResponse {
    SettleResponse::Success {
        payer: Some("0xpayer".into()),
        transaction: "0xabc".into(),
        network: "eip155:8453".into(),
        amount: Some("1000000".into()),
        extensions: Extensions::new(),
        extension_responses: Extensions::new(),
        extra: None,
    }
}

async fn mount_supported(server: &MockServer) {
    mount_supported_body(server, supported_body()).await;
}

async fn mount_supported_body(server: &MockServer, body: SupportedResponse) {
    Mock::given(method("GET"))
        .and(path("/supported"))
        .respond_with(ResponseTemplate::new(200).set_body_json(body))
        .mount(server)
        .await;
}

async fn mock_facilitator_ok() -> MockServer {
    let server = MockServer::start().await;
    mount_supported(&server).await;
    Mock::given(method("POST"))
        .and(path("/verify"))
        .respond_with(ResponseTemplate::new(200).set_body_json(VerifyResponse::valid("0xpayer")))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/settle"))
        .respond_with(ResponseTemplate::new(200).set_body_json(settle_ok()))
        .mount(&server)
        .await;
    server
}

async fn mock_facilitator_permit2_412() -> MockServer {
    let server = MockServer::start().await;
    mount_supported(&server).await;
    Mock::given(method("POST"))
        .and(path("/verify"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(VerifyResponse::invalid(
                None,
                ErrorReason::Permit2AllowanceRequired,
            )),
        )
        .mount(&server)
        .await;
    server
}

async fn mock_facilitator_supported_transport() -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/supported"))
        .respond_with(ResponseTemplate::new(500))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/verify"))
        .respond_with(ResponseTemplate::new(404))
        .expect(0)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/settle"))
        .respond_with(ResponseTemplate::new(404))
        .expect(0)
        .mount(&server)
        .await;
    server
}

async fn ok_handler() -> &'static str {
    "ok"
}

async fn serve_paid(fac_uri: &str, tag: r402_protocol::payment::PriceTag) -> Url {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let origin: Url = format!("http://{addr}").parse().unwrap();
    let layer = X402Middleware::try_new(fac_uri)
        .expect("facilitator url")
        .with_base_url(origin.clone())
        .with_scheme(ChainIdPattern::wildcard("eip155"), Eip155Exact)
        .with_price_tag(tag)
        .expect("price tag")
        .with_settlement_mode(SettlementMode::Sequential)
        .expect("compatible settlement mode");
    let app = Router::new().route("/paid", get(ok_handler).layer(layer));
    tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve");
    });
    origin
}

async fn serve_paid_in_process(
    fac: Eip155ExactFacilitator<Eip155ChainProvider>,
    tag: r402_protocol::payment::PriceTag,
) -> Url {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let origin: Url = format!("http://{addr}").parse().unwrap();
    let layer = X402Middleware::from_facilitator(fac)
        .with_base_url(origin.clone())
        .with_scheme(ChainIdPattern::wildcard("eip155"), Eip155Exact)
        .with_price_tag(tag)
        .expect("price tag")
        .with_settlement_mode(SettlementMode::Sequential)
        .expect("compatible settlement mode");
    let app = Router::new().route("/paid", get(ok_handler).layer(layer));
    tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve");
    });
    origin
}

fn paid_url(origin: &Url) -> String {
    format!("{origin}paid")
}

fn upto_tag() -> r402_protocol::payment::PriceTag {
    Eip155Upto::price_tag(pay_to(), USDC::base().amount(1_000_000u64))
}

async fn serve_upto(fac_uri: &str, tag: r402_protocol::payment::PriceTag) -> Url {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let origin: Url = format!("http://{addr}").parse().unwrap();
    let layer = X402Middleware::try_new(fac_uri)
        .expect("facilitator url")
        .with_base_url(origin.clone())
        .with_scheme(ChainIdPattern::wildcard("eip155"), Eip155Upto)
        .with_price_tag(tag)
        .expect("price tag")
        .with_settlement_mode(SettlementMode::Sequential)
        .expect("compatible settlement mode");
    let app = Router::new().route("/paid", get(ok_handler).layer(layer));
    tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve");
    });
    origin
}

async fn serve_dynamic_exact(fac_uri: &str, tag: r402_protocol::payment::PriceTag) -> Url {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let origin: Url = format!("http://{addr}").parse().unwrap();
    let layer = X402Middleware::try_new(fac_uri)
        .expect("facilitator url")
        .with_base_url(origin.clone())
        .with_scheme(ChainIdPattern::wildcard("eip155"), Eip155Exact)
        .with_dynamic_price(move |_, _, _| {
            let tag = tag.clone();
            std::future::ready(vec![tag])
        })
        .expect("dynamic price");
    let app = Router::new().route("/paid", get(ok_handler).layer(layer));
    tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve");
    });
    origin
}

async fn mock_supported_only(scheme: &str, extensions: &[&str]) -> MockServer {
    let server = MockServer::start().await;
    mount_supported_body(&server, supported_body_for(scheme, extensions)).await;
    server
}

fn decode_required(response: &reqwest::Response) -> serde_json::Value {
    let header = response.headers().get(PAYMENT_REQUIRED).unwrap();
    let decoded = Base64Bytes::from(header.as_bytes()).decode().unwrap();
    serde_json::from_slice(&decoded).unwrap()
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
async fn unpaid_without_header_is_402_with_payment_required() {
    let fac = mock_facilitator_ok().await;
    let origin = serve_paid(&fac.uri(), eip3009_tag()).await;
    let response = http_client().get(paid_url(&origin)).send().await.unwrap();
    assert_eq!(response.status(), StatusCode::PAYMENT_REQUIRED);
    assert!(
        response.headers().get(PAYMENT_REQUIRED).is_some(),
        "unpaid Sequential with_price_tag must emit Payment-Required before the buyer retries"
    );
    let body = decode_required(&response);
    assert_ext(&body, EIP2612_GAS_SPONSORING_KEY, false);
    assert_ext(&body, ERC20_APPROVAL_GAS_SPONSORING_KEY, false);
}

#[tokio::test]
async fn unpaid_then_pay_is_200_with_payment_response() {
    let fac = mock_facilitator_ok().await;
    let origin = serve_paid(&fac.uri(), eip3009_tag()).await;
    let client = http_client().with_payments(paid_buyer());
    let response = client.get(paid_url(&origin)).send().await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert!(
        response.headers().get(PAYMENT_RESPONSE).is_some(),
        "success must attach Payment-Response"
    );
    assert_eq!(response.text().await.unwrap(), "ok");
}

#[tokio::test]
async fn permit2_allowance_required_is_412() {
    let fac = mock_facilitator_permit2_412().await;
    let origin = serve_paid(&fac.uri(), permit2_tag()).await;
    let client = http_client().with_payments(paid_buyer());
    let response = client.get(paid_url(&origin)).send().await.unwrap();
    assert_eq!(
        response.status(),
        StatusCode::PRECONDITION_FAILED,
        "412 Permit2 must be returned without a paid retry"
    );
}

#[tokio::test]
async fn facilitator_transport_is_502() {
    let fac = mock_facilitator_supported_transport().await;
    let origin = serve_paid(&fac.uri(), eip3009_tag()).await;
    let response = http_client().get(paid_url(&origin)).send().await.unwrap();
    assert_eq!(
        response.status(),
        StatusCode::BAD_GATEWAY,
        "supported transport must 502 on the first unpaid response"
    );
    assert!(
        response.headers().get(PAYMENT_REQUIRED).is_none(),
        "502 must not carry Payment-Required"
    );
}

#[tokio::test]
async fn malformed_signature_is_402() {
    let fac = mock_facilitator_ok().await;
    let origin = serve_paid(&fac.uri(), eip3009_tag()).await;
    let response = http_client()
        .get(paid_url(&origin))
        .header(PAYMENT_SIGNATURE, "not-valid-base64!!!")
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::PAYMENT_REQUIRED);
    assert!(
        response.headers().get(PAYMENT_REQUIRED).is_some(),
        "malformed Payment-Signature must still emit Payment-Required"
    );
}

#[tokio::test]
async fn upto_402_has_2612_without_with_extension() {
    let fac = mock_supported_only("upto", &[]).await;
    let origin = serve_upto(&fac.uri(), upto_tag()).await;
    let response = http_client().get(paid_url(&origin)).send().await.unwrap();
    assert_eq!(response.status(), StatusCode::PAYMENT_REQUIRED);
    let body = decode_required(&response);
    assert_ext(&body, EIP2612_GAS_SPONSORING_KEY, true);
    assert_ext(&body, ERC20_APPROVAL_GAS_SPONSORING_KEY, false);
}

#[tokio::test]
async fn upto_402_has_erc20_only_when_supported_lists_it() {
    let fac = mock_supported_only("upto", &[ERC20_APPROVAL_GAS_SPONSORING_KEY]).await;
    let origin = serve_upto(&fac.uri(), upto_tag()).await;
    let response = http_client().get(paid_url(&origin)).send().await.unwrap();
    assert_eq!(response.status(), StatusCode::PAYMENT_REQUIRED);
    let body = decode_required(&response);
    assert_ext(&body, EIP2612_GAS_SPONSORING_KEY, true);
    assert_ext(&body, ERC20_APPROVAL_GAS_SPONSORING_KEY, true);
}

#[tokio::test]
async fn eip3009_402_omits_keys_even_when_supported_lists_them() {
    let fac = mock_supported_only(
        "exact",
        &[
            EIP2612_GAS_SPONSORING_KEY,
            ERC20_APPROVAL_GAS_SPONSORING_KEY,
        ],
    )
    .await;
    let origin = serve_paid(&fac.uri(), eip3009_tag()).await;
    let response = http_client().get(paid_url(&origin)).send().await.unwrap();
    assert_eq!(response.status(), StatusCode::PAYMENT_REQUIRED);
    let body = decode_required(&response);
    assert_ext(&body, EIP2612_GAS_SPONSORING_KEY, false);
    assert_ext(&body, ERC20_APPROVAL_GAS_SPONSORING_KEY, false);
}

#[tokio::test]
async fn exact_permit2_dynamic_price_auto_inserts_2612() {
    let fac = mock_supported_only("exact", &[]).await;
    let origin = serve_dynamic_exact(&fac.uri(), permit2_tag()).await;
    let response = http_client().get(paid_url(&origin)).send().await.unwrap();
    assert_eq!(response.status(), StatusCode::PAYMENT_REQUIRED);
    let body = decode_required(&response);
    assert_ext(&body, EIP2612_GAS_SPONSORING_KEY, true);
    assert_ext(&body, ERC20_APPROVAL_GAS_SPONSORING_KEY, false);
}

#[tokio::test]
async fn exact_permit2_dynamic_price_erc20_iff_supported_lists_it() {
    let fac = mock_supported_only("exact", &[ERC20_APPROVAL_GAS_SPONSORING_KEY]).await;
    let origin = serve_dynamic_exact(&fac.uri(), permit2_tag()).await;
    let response = http_client().get(paid_url(&origin)).send().await.unwrap();
    assert_eq!(response.status(), StatusCode::PAYMENT_REQUIRED);
    let body = decode_required(&response);
    assert_ext(&body, EIP2612_GAS_SPONSORING_KEY, true);
    assert_ext(&body, ERC20_APPROVAL_GAS_SPONSORING_KEY, true);
}

#[tokio::test]
async fn middleware_with_extension_advertises_erc20_when_remote_omits_it() {
    let fac = mock_supported_only("upto", &[]).await;
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let origin: Url = format!("http://{addr}").parse().unwrap();
    let layer = X402Middleware::try_new(&fac.uri())
        .expect("facilitator url")
        .with_extension(Erc20ApprovalGasSponsoringExtension::new())
        .with_base_url(origin.clone())
        .with_scheme(ChainIdPattern::wildcard("eip155"), Eip155Upto)
        .with_price_tag(upto_tag())
        .expect("price tag");
    let app = Router::new().route("/paid", get(ok_handler).layer(layer));
    tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve");
    });
    let response = http_client().get(paid_url(&origin)).send().await.unwrap();
    assert_eq!(response.status(), StatusCode::PAYMENT_REQUIRED);
    let body = decode_required(&response);
    assert_ext(&body, EIP2612_GAS_SPONSORING_KEY, true);
    assert_ext(&body, ERC20_APPROVAL_GAS_SPONSORING_KEY, true);
}

#[tokio::test]
async fn layer_with_extension_after_price_tag_advertises_erc20() {
    let fac = mock_supported_only("upto", &[]).await;
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let origin: Url = format!("http://{addr}").parse().unwrap();
    let layer = X402Middleware::try_new(&fac.uri())
        .expect("facilitator url")
        .with_base_url(origin.clone())
        .with_scheme(ChainIdPattern::wildcard("eip155"), Eip155Upto)
        .with_price_tag(upto_tag())
        .expect("price tag")
        .with_extension(Erc20ApprovalGasSponsoringExtension::new());
    let app = Router::new().route("/paid", get(ok_handler).layer(layer));
    tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve");
    });
    let response = http_client().get(paid_url(&origin)).send().await.unwrap();
    assert_eq!(response.status(), StatusCode::PAYMENT_REQUIRED);
    let body = decode_required(&response);
    assert_ext(&body, EIP2612_GAS_SPONSORING_KEY, true);
    assert_ext(&body, ERC20_APPROVAL_GAS_SPONSORING_KEY, true);
}

fn anvil_wallet() -> EthereumWallet {
    let signer = PrivateKeySigner::from_str(ANVIL_KEY).expect("anvil key");
    EthereumWallet::from(signer)
}

async fn anvil_facilitator() -> Eip155ExactFacilitator<Eip155ChainProvider> {
    let url = Url::parse(ANVIL_RPC).expect("anvil rpc");
    let provider = Eip155ChainProvider::new(
        Eip155ChainReference::new(8453),
        anvil_wallet(),
        &[(url, None)],
        true,
        false,
        15,
    )
    .expect("provider");
    let inner = Eip155MetaTransactionProvider::inner(&provider);
    tokio::time::timeout(std::time::Duration::from_secs(2), inner.get_chain_id())
        .await
        .expect("R402_ANVIL=1 but Anvil did not respond at http://127.0.0.1:8545")
        .expect("anvil eth_chainId");
    Eip155ExactFacilitator::try_new(provider).expect("try_new")
}

#[tokio::test]
async fn anvil_in_process_facilitator_unpaid_is_402() {
    if !anvil_opted_in() {
        eprintln!("R402_ANVIL!=1 — skipping in-process Anvil facilitator E2E");
        return;
    }
    let fac = anvil_facilitator().await;
    let origin = serve_paid_in_process(fac, eip3009_tag()).await;
    let response = http_client().get(paid_url(&origin)).send().await.unwrap();
    assert_eq!(response.status(), StatusCode::PAYMENT_REQUIRED);
    assert!(
        response.headers().get(PAYMENT_REQUIRED).is_some(),
        "Anvil in-process facilitator unpaid path must emit Payment-Required"
    );
}
