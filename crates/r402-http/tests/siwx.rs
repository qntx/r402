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

//! SIWX access-grant: 402-carried challenge, never Host, zero `/verify`.

mod harness;

use std::cell::RefCell;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex, Once};
use std::time::Duration;

use alloy_signer::Signer;
use alloy_signer_local::PrivateKeySigner;
use axum_core::body::Body;
use axum_core::extract::Request;
use compact_str::CompactString;
use http::{HeaderValue, StatusCode};
use r402_client::ClientExtension;
use r402_extensions::siwx::DEFAULT_STATEMENT;
use r402_http::server::{
    BackgroundSettlementTracker, BuildError, InMemoryPaidAddressStore, PAYMENT_REQUIRED,
    PAYMENT_RESPONSE, PaidAddressStore, SIGN_IN_WITH_X, SIWX_KEY, SettlementMode, SiwxChain,
    SiwxClientExtension, SiwxError, SiwxGate, SiwxOrigin, SiwxProof, SiwxSigner,
};
use r402_protocol::payment::{Base64Bytes, PaymentRequired};
use serde_json::Value;
use tracing::field::{Field, Visit};
use tracing::span::{Attributes, Id, Record};
use tracing::subscriber::Interest;
use tracing::{Event, Metadata, Subscriber};

use crate::harness::{
    FakeFacilitator, FlowScheme, OkInner, SettleScript, call_layer, eip155_requirements,
    eip155_tag, middleware, payment_request, unpaid_request,
};

const FIXTURE_KEY: &str = "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80";

fn origin() -> SiwxOrigin {
    SiwxOrigin::parse("https://api.example.com").unwrap()
}

fn gate(store: InMemoryPaidAddressStore) -> SiwxGate {
    SiwxGate::new(origin(), store).with_chain(SiwxChain::eip191("eip155:8453"))
}

fn decode_required(response: &axum_core::response::Response) -> Value {
    let header = response.headers().get(PAYMENT_REQUIRED).unwrap();
    let decoded = Base64Bytes::from(header.as_bytes()).decode().unwrap();
    serde_json::from_slice(&decoded).unwrap()
}

async fn sign_challenge(info: &Value) -> (String, HeaderValue) {
    let signer: PrivateKeySigner = FIXTURE_KEY.parse().unwrap();
    let address = format!("{}", signer.address());
    let mut body = info.clone();
    body["address"] = address.clone().into();
    body["chainId"] = "eip155:8453".into();
    body["type"] = "eip191".into();
    body["signature"] = "00".into();
    let header = Base64Bytes::encode(serde_json::to_vec(&body).unwrap()).to_string();
    let mut proof = SiwxProof::parse_header(&header).unwrap();
    let raw = proof.signing_message().unwrap();
    let sig = signer.sign_message(raw.as_bytes()).await.unwrap();
    proof.signature = format!("{sig}").into();
    body["signature"] = proof.signature.to_string().into();
    let encoded = Base64Bytes::encode(serde_json::to_vec(&body).unwrap());
    (address, HeaderValue::from_bytes(encoded.as_ref()).unwrap())
}

fn siwx_request(header: HeaderValue, host: &str) -> Request {
    Request::builder()
        .uri("/paid")
        .header("host", host)
        .header(SIGN_IN_WITH_X, header)
        .body(Body::empty())
        .unwrap()
}

#[tokio::test]
async fn unpaid_402_carries_challenge_from_configured_origin() {
    let fac = Arc::new(FakeFacilitator::new());
    let store = InMemoryPaidAddressStore::new();
    let layer = middleware(Arc::clone(&fac), FlowScheme::authorization())
        .with_siwx(gate(store))
        .with_price_tag(eip155_tag())
        .unwrap()
        .with_settlement_mode(SettlementMode::Sequential)
        .expect("compatible settlement mode");
    let mut req = unpaid_request();
    let _ = req
        .headers_mut()
        .insert("host", HeaderValue::from_static("evil.example"));
    let response = call_layer(layer, OkInner, req).await;
    assert_eq!(response.status(), StatusCode::PAYMENT_REQUIRED);
    let body = decode_required(&response);
    let info = &body["extensions"][SIWX_KEY]["info"];
    assert_eq!(info["domain"], "api.example.com");
    assert_eq!(info["uri"], "https://api.example.com/paid");
    assert_eq!(info["statement"], DEFAULT_STATEMENT);
    let nonce = info["nonce"].as_str().unwrap();
    assert_eq!(nonce.len(), 32);
    assert_eq!(fac.verify_count(), 0);
}

#[tokio::test]
async fn challenge_statement_is_default() {
    let fac = Arc::new(FakeFacilitator::new());
    let store = InMemoryPaidAddressStore::new();
    let layer = middleware(Arc::clone(&fac), FlowScheme::authorization())
        .with_siwx(gate(store))
        .with_price_tag(eip155_tag())
        .unwrap();
    let response = call_layer(layer, OkInner, unpaid_request()).await;
    assert_eq!(response.status(), StatusCode::PAYMENT_REQUIRED);
    let info = &decode_required(&response)["extensions"][SIWX_KEY]["info"];
    assert_eq!(info["statement"], DEFAULT_STATEMENT);
    assert_eq!(fac.verify_count(), 0);
}

#[tokio::test]
async fn grant_access_when_paid_skips_verify() {
    let fac = Arc::new(FakeFacilitator::new());
    let store = InMemoryPaidAddressStore::new();
    let layer = middleware(Arc::clone(&fac), FlowScheme::authorization())
        .with_siwx(gate(store.clone()))
        .with_price_tag(eip155_tag())
        .unwrap();
    let challenge = call_layer(layer.clone(), OkInner, unpaid_request()).await;
    let info = decode_required(&challenge)["extensions"][SIWX_KEY]["info"].clone();
    let (address, header) = sign_challenge(&info).await;
    store.record_success(origin().store_key("/paid").as_str(), &address);
    let response = call_layer(layer, OkInner, siwx_request(header, "evil.example")).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(fac.verify_count(), 0);
    assert_eq!(fac.settle_count(), 0);
    assert!(response.headers().get(PAYMENT_REQUIRED).is_none());
}

#[tokio::test]
async fn valid_proof_without_payment_history_is_402() {
    let fac = Arc::new(FakeFacilitator::new());
    let store = InMemoryPaidAddressStore::new();
    let layer = middleware(Arc::clone(&fac), FlowScheme::authorization())
        .with_siwx(gate(store))
        .with_price_tag(eip155_tag())
        .unwrap();
    let challenge = call_layer(layer.clone(), OkInner, unpaid_request()).await;
    let info = decode_required(&challenge)["extensions"][SIWX_KEY]["info"].clone();
    let (_, header) = sign_challenge(&info).await;
    let response = call_layer(layer, OkInner, siwx_request(header, "api.example.com")).await;
    assert_eq!(response.status(), StatusCode::PAYMENT_REQUIRED);
    assert_eq!(fac.verify_count(), 0);
}

#[tokio::test]
async fn unpaid_valid_proof_retries_after_store_hit() {
    let fac = Arc::new(FakeFacilitator::new());
    let store = InMemoryPaidAddressStore::new();
    let layer = middleware(Arc::clone(&fac), FlowScheme::authorization())
        .with_siwx(gate(store.clone()))
        .with_price_tag(eip155_tag())
        .unwrap();
    let challenge = call_layer(layer.clone(), OkInner, unpaid_request()).await;
    let info = decode_required(&challenge)["extensions"][SIWX_KEY]["info"].clone();
    let (address, header) = sign_challenge(&info).await;
    let miss = call_layer(
        layer.clone(),
        OkInner,
        siwx_request(header.clone(), "api.example.com"),
    )
    .await;
    assert_eq!(miss.status(), StatusCode::PAYMENT_REQUIRED);
    store.record_success(origin().store_key("/paid").as_str(), &address);
    let retry = call_layer(layer, OkInner, siwx_request(header, "api.example.com")).await;
    assert_eq!(retry.status(), StatusCode::OK);
    assert_eq!(fac.verify_count(), 0);
}

#[tokio::test]
async fn auth_only_grants_without_store() {
    let fac = Arc::new(FakeFacilitator::new());
    let store = InMemoryPaidAddressStore::new();
    let layer = middleware(Arc::clone(&fac), FlowScheme::authorization())
        .with_auth_only(gate(store))
        .with_price_tag(eip155_tag())
        .unwrap();
    let challenge = call_layer(layer.clone(), OkInner, unpaid_request()).await;
    let info = decode_required(&challenge)["extensions"][SIWX_KEY]["info"].clone();
    let (_, header) = sign_challenge(&info).await;
    let response = call_layer(layer, OkInner, siwx_request(header, "evil.example")).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(fac.verify_count(), 0);
    assert_eq!(fac.settle_count(), 0);
}

#[tokio::test]
async fn auth_only_empty_tags_unpaid_is_402() {
    let fac = Arc::new(FakeFacilitator::new());
    let store = InMemoryPaidAddressStore::new();
    let layer = middleware(Arc::clone(&fac), FlowScheme::authorization())
        .with_auth_only(gate(store))
        .with_price_tags(vec![])
        .unwrap();
    let response = call_layer(layer, OkInner, unpaid_request()).await;
    assert_eq!(response.status(), StatusCode::PAYMENT_REQUIRED);
    let body = decode_required(&response);
    assert!(body["extensions"].get(SIWX_KEY).is_some());
    assert_eq!(fac.verify_count(), 0);
}

#[test]
fn empty_tags_before_auth_only_is_err() {
    let fac = Arc::new(FakeFacilitator::new());
    let err = middleware(Arc::clone(&fac), FlowScheme::authorization())
        .with_price_tags(vec![])
        .expect_err("empty tags without auth_only");
    assert_eq!(err, BuildError::EmptyPriceTags);
}

#[test]
fn empty_tags_then_layer_with_siwx_clears_auth_only_is_err() {
    let fac = Arc::new(FakeFacilitator::new());
    let store = InMemoryPaidAddressStore::new();
    let layer = middleware(Arc::clone(&fac), FlowScheme::authorization())
        .with_auth_only(gate(store.clone()))
        .with_price_tags(vec![])
        .unwrap();
    let err = layer
        .with_siwx(gate(store))
        .expect_err("replacing SIWX without auth_only on empty tags");
    assert_eq!(err, BuildError::EmptyPriceTags);
}

#[tokio::test]
async fn empty_tags_layer_with_auth_only_stays_ok() {
    let fac = Arc::new(FakeFacilitator::new());
    let store = InMemoryPaidAddressStore::new();
    let layer = middleware(Arc::clone(&fac), FlowScheme::authorization())
        .with_auth_only(gate(store.clone()))
        .with_price_tags(vec![])
        .unwrap()
        .with_auth_only(gate(store))
        .unwrap();
    let response = call_layer(layer, OkInner, unpaid_request()).await;
    assert_eq!(response.status(), StatusCode::PAYMENT_REQUIRED);
    let body = decode_required(&response);
    assert!(body["extensions"].get(SIWX_KEY).is_some());
}

#[tokio::test]
async fn reused_nonce_is_402() {
    let fac = Arc::new(FakeFacilitator::new());
    let store = InMemoryPaidAddressStore::new();
    let layer = middleware(Arc::clone(&fac), FlowScheme::authorization())
        .with_auth_only(gate(store))
        .with_price_tag(eip155_tag())
        .unwrap();
    let challenge = call_layer(layer.clone(), OkInner, unpaid_request()).await;
    let info = decode_required(&challenge)["extensions"][SIWX_KEY]["info"].clone();
    let (_, header) = sign_challenge(&info).await;
    let first = call_layer(
        layer.clone(),
        OkInner,
        siwx_request(header.clone(), "api.example.com"),
    )
    .await;
    assert_eq!(first.status(), StatusCode::OK);
    let second = call_layer(layer, OkInner, siwx_request(header, "api.example.com")).await;
    assert_eq!(second.status(), StatusCode::PAYMENT_REQUIRED);
    assert_eq!(fac.verify_count(), 0);
}

#[tokio::test]
async fn invalid_signature_is_402_zero_verify() {
    let fac = Arc::new(FakeFacilitator::new());
    let store = InMemoryPaidAddressStore::new();
    let layer = middleware(Arc::clone(&fac), FlowScheme::authorization())
        .with_auth_only(gate(store))
        .with_price_tag(eip155_tag())
        .unwrap();
    let challenge = call_layer(layer.clone(), OkInner, unpaid_request()).await;
    let mut info = decode_required(&challenge)["extensions"][SIWX_KEY]["info"].clone();
    info["address"] = "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266".into();
    info["chainId"] = "eip155:8453".into();
    info["type"] = "eip191".into();
    info["signature"] = "0x1111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111".into();
    let encoded = Base64Bytes::encode(serde_json::to_vec(&info).unwrap());
    let header = HeaderValue::from_bytes(encoded.as_ref()).unwrap();
    let response = call_layer(layer, OkInner, siwx_request(header, "api.example.com")).await;
    assert_eq!(response.status(), StatusCode::PAYMENT_REQUIRED);
    assert_eq!(fac.verify_count(), 0);
}

#[tokio::test]
async fn invalid_proof_does_not_burn_nonce() {
    let fac = Arc::new(FakeFacilitator::new());
    let store = InMemoryPaidAddressStore::new();
    let layer = middleware(Arc::clone(&fac), FlowScheme::authorization())
        .with_auth_only(gate(store))
        .with_price_tag(eip155_tag())
        .unwrap();
    let challenge = call_layer(layer.clone(), OkInner, unpaid_request()).await;
    let info = decode_required(&challenge)["extensions"][SIWX_KEY]["info"].clone();
    let mut bad = info.clone();
    bad["address"] = "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266".into();
    bad["chainId"] = "eip155:8453".into();
    bad["type"] = "eip191".into();
    bad["signature"] = "0x1111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111".into();
    let bad_header =
        HeaderValue::from_bytes(Base64Bytes::encode(serde_json::to_vec(&bad).unwrap()).as_ref())
            .unwrap();
    let dummy = call_layer(
        layer.clone(),
        OkInner,
        siwx_request(bad_header, "api.example.com"),
    )
    .await;
    assert_eq!(dummy.status(), StatusCode::PAYMENT_REQUIRED);
    let (_, good_header) = sign_challenge(&info).await;
    let retry = call_layer(layer, OkInner, siwx_request(good_header, "api.example.com")).await;
    assert_eq!(retry.status(), StatusCode::OK);
    assert_eq!(fac.verify_count(), 0);
}

#[tokio::test]
async fn concurrent_replay_grants_once() {
    let fac = Arc::new(FakeFacilitator::new());
    let store = InMemoryPaidAddressStore::new();
    let layer = middleware(Arc::clone(&fac), FlowScheme::authorization())
        .with_auth_only(gate(store))
        .with_price_tag(eip155_tag())
        .unwrap();
    let challenge = call_layer(layer.clone(), OkInner, unpaid_request()).await;
    let info = decode_required(&challenge)["extensions"][SIWX_KEY]["info"].clone();
    let (_, header) = sign_challenge(&info).await;
    let (a, b) = tokio::join!(
        call_layer(
            layer.clone(),
            OkInner,
            siwx_request(header.clone(), "api.example.com"),
        ),
        call_layer(layer, OkInner, siwx_request(header, "api.example.com")),
    );
    let ok = [a.status(), b.status()]
        .into_iter()
        .filter(|s| *s == StatusCode::OK)
        .count();
    assert_eq!(ok, 1, "one replay must GrantAccess, the other 402");
    assert_eq!(fac.verify_count(), 0);
}

#[tokio::test]
async fn settle_success_records_payer() {
    let fac = Arc::new(FakeFacilitator::new());
    let store = InMemoryPaidAddressStore::new();
    let layer = middleware(Arc::clone(&fac), FlowScheme::authorization())
        .with_siwx(gate(store.clone()))
        .with_price_tag(eip155_tag())
        .unwrap();
    let response = call_layer(layer, OkInner, payment_request(&eip155_requirements())).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert!(response.headers().get(PAYMENT_RESPONSE).is_some());
    assert!(store.contains(origin().store_key("/paid").as_str(), "0xpayer"));
}

#[tokio::test]
async fn background_settle_success_records_payer() {
    let fac = Arc::new(FakeFacilitator::new());
    let store = InMemoryPaidAddressStore::new();
    let tracker = BackgroundSettlementTracker::new();
    let layer = middleware(Arc::clone(&fac), FlowScheme::authorization())
        .with_siwx(gate(store.clone()))
        .with_price_tag(eip155_tag())
        .unwrap()
        .with_settlement_mode(SettlementMode::Background)
        .expect("compatible settlement mode")
        .with_settlement_tracker(tracker.clone());
    let response = call_layer(layer, OkInner, payment_request(&eip155_requirements())).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert!(
        response.headers().get(PAYMENT_RESPONSE).is_none(),
        "Background must not attach Payment-Response"
    );
    tracker
        .wait_for_drain(Duration::from_secs(1))
        .await
        .expect("background settle drains");
    assert!(
        store.contains(origin().store_key("/paid").as_str(), "0xpayer"),
        "spawned settle Success must record PaidAddressStore"
    );
}

#[tokio::test]
async fn background_settle_failure_does_not_record() {
    let fac = Arc::new(FakeFacilitator::new());
    *fac.settle.lock().unwrap() = SettleScript::Fail;
    let store = InMemoryPaidAddressStore::new();
    let tracker = BackgroundSettlementTracker::new();
    let layer = middleware(Arc::clone(&fac), FlowScheme::authorization())
        .with_siwx(gate(store.clone()))
        .with_price_tag(eip155_tag())
        .unwrap()
        .with_settlement_mode(SettlementMode::Background)
        .expect("compatible settlement mode")
        .with_settlement_tracker(tracker.clone());
    let response = call_layer(layer, OkInner, payment_request(&eip155_requirements())).await;
    assert_eq!(response.status(), StatusCode::OK);
    tracker
        .wait_for_drain(Duration::from_secs(1))
        .await
        .expect("failed background settle drains");
    assert!(!store.contains(origin().store_key("/paid").as_str(), "0xpayer"));
}

struct FixtureEvm(PrivateKeySigner);

impl SiwxSigner for FixtureEvm {
    fn signature_type(&self) -> &'static str {
        "eip191"
    }

    fn address(&self) -> CompactString {
        format!("{}", self.0.address()).into()
    }

    fn sign_message<'a>(
        &'a self,
        message: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<CompactString, SiwxError>> + Send + 'a>> {
        Box::pin(async move {
            let sig = self
                .0
                .sign_message(message.as_bytes())
                .await
                .map_err(|_| SiwxError::Signature)?;
            Ok(format!("{sig}").into())
        })
    }
}

#[tokio::test]
async fn client_extension_signs_configured_origin_not_host() {
    let fac = Arc::new(FakeFacilitator::new());
    let store = InMemoryPaidAddressStore::new();
    let layer = middleware(Arc::clone(&fac), FlowScheme::authorization())
        .with_siwx(gate(store.clone()))
        .with_price_tag(eip155_tag())
        .unwrap();
    let mut challenge_req = unpaid_request();
    let _ = challenge_req
        .headers_mut()
        .insert("host", HeaderValue::from_static("evil.example"));
    let challenge = call_layer(layer.clone(), OkInner, challenge_req).await;
    assert_eq!(challenge.status(), StatusCode::PAYMENT_REQUIRED);
    let mut required: PaymentRequired = serde_json::from_slice(
        &Base64Bytes::from(
            challenge
                .headers()
                .get(PAYMENT_REQUIRED)
                .unwrap()
                .as_bytes(),
        )
        .decode()
        .unwrap(),
    )
    .unwrap();
    assert_eq!(
        required.extensions.get(SIWX_KEY).unwrap().to_value()["info"]["domain"],
        "api.example.com"
    );

    let signer: PrivateKeySigner = FIXTURE_KEY.parse().unwrap();
    let address = format!("{}", signer.address());
    let ext = SiwxClientExtension::new().with_signer(FixtureEvm(signer));
    required.resource.url = "https://evil.example/paid".into();
    let headers = ext
        .on_payment_required(&required, "https://api.example.com/paid")
        .await;
    let header = headers.get(SIGN_IN_WITH_X).expect("SIGN-IN-WITH-X").clone();
    store.record_success(origin().store_key("/paid").as_str(), &address);
    let response = call_layer(layer, OkInner, siwx_request(header, "evil.example")).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(fac.verify_count(), 0);
}

#[tokio::test]
async fn client_extension_skips_origin_mismatch() {
    let signer: PrivateKeySigner = FIXTURE_KEY.parse().unwrap();
    let ext = SiwxClientExtension::new().with_signer(FixtureEvm(signer));
    let mut required = PaymentRequired::new(r402_protocol::payment::ResourceInfo::new(
        "https://api.example.com/paid",
    ));
    required.extensions.insert(
        SIWX_KEY,
        r402_protocol::payment::ExtensionEntry::raw(serde_json::json!({
            "info": {
                "domain": "evil.example.com",
                "uri": "https://evil.example.com/paid",
                "version": "1",
                "nonce": "a1b2c3d4e5f67890a1b2c3d4e5f67890",
                "issuedAt": "2024-01-15T10:30:00.000Z"
            },
            "supportedChains": [{"chainId": "eip155:8453", "type": "eip191"}]
        })),
    );
    let headers = ext
        .on_payment_required(&required, "https://api.example.com/paid")
        .await;
    assert!(
        headers.get(SIGN_IN_WITH_X).is_none(),
        "mismatched challenge origin must not sign"
    );
}

#[derive(Clone, Debug, Default)]
struct LogCapture {
    events: Arc<Mutex<Vec<CapturedEvent>>>,
}

#[derive(Debug)]
struct CapturedEvent {
    message: String,
    fields: Vec<(String, String)>,
}

#[derive(Debug)]
struct FieldVisitor {
    message: String,
    fields: Vec<(String, String)>,
}

impl Visit for FieldVisitor {
    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        let rendered = format!("{value:?}");
        if field.name() == "message" {
            rendered.trim_matches('"').clone_into(&mut self.message);
        } else {
            self.fields.push((field.name().to_owned(), rendered));
        }
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == "message" {
            value.clone_into(&mut self.message);
        } else {
            self.fields
                .push((field.name().to_owned(), value.to_owned()));
        }
    }
}

impl LogCapture {
    fn dump(&self) -> String {
        let events = self.events.lock().expect("log capture lock");
        events
            .iter()
            .map(|event| {
                let fields = event
                    .fields
                    .iter()
                    .map(|(key, value)| format!("{key}={value}"))
                    .collect::<Vec<_>>()
                    .join(" ");
                if fields.is_empty() {
                    event.message.clone()
                } else {
                    format!("{} {fields}", event.message)
                }
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn messages(&self) -> Vec<String> {
        self.events
            .lock()
            .expect("log capture lock")
            .iter()
            .map(|event| event.message.clone())
            .collect()
    }

    fn record_event(&self, event: &Event<'_>) {
        let mut visitor = FieldVisitor {
            message: String::new(),
            fields: Vec::new(),
        };
        event.record(&mut visitor);
        self.events
            .lock()
            .expect("log capture lock")
            .push(CapturedEvent {
                message: visitor.message,
                fields: visitor.fields,
            });
    }
}

const SIWX_LOG_TARGET: &str = "r402_http::server::siwx";

thread_local! {
    static CAPTURE: RefCell<Option<LogCapture>> = const { RefCell::new(None) };
}

/// `set_default` races tracing's process-global callsite interest cache:
/// a parallel test dropping its guard can rebuild `siwx` debug events to
/// `Interest::never`, so the capturing test records `[]`.
struct SiwxTestSubscriber;

impl Subscriber for SiwxTestSubscriber {
    fn register_callsite(&self, metadata: &Metadata<'_>) -> Interest {
        if metadata.target() == SIWX_LOG_TARGET {
            Interest::always()
        } else {
            Interest::never()
        }
    }

    fn enabled(&self, metadata: &Metadata<'_>) -> bool {
        metadata.target() == SIWX_LOG_TARGET
    }

    fn new_span(&self, _span: &Attributes<'_>) -> Id {
        Id::from_u64(1)
    }

    fn record(&self, _span: &Id, _values: &Record<'_>) {}

    fn record_follows_from(&self, _span: &Id, _follows: &Id) {}

    fn event(&self, event: &Event<'_>) {
        if event.metadata().target() != SIWX_LOG_TARGET {
            return;
        }
        CAPTURE.with(|slot| {
            if let Some(capture) = slot.borrow().as_ref() {
                capture.record_event(event);
            }
        });
    }

    fn enter(&self, _span: &Id) {}

    fn exit(&self, _span: &Id) {}
}

struct CaptureGuard;

impl Drop for CaptureGuard {
    fn drop(&mut self) {
        CAPTURE.with(|slot| {
            slot.replace(None);
        });
    }
}

fn install_siwx_test_subscriber() {
    static INSTALL: Once = Once::new();
    INSTALL.call_once(|| {
        tracing::subscriber::set_global_default(SiwxTestSubscriber)
            .expect("siwx tests set the process tracing subscriber once");
    });
}

fn capture_logs() -> (LogCapture, CaptureGuard) {
    install_siwx_test_subscriber();
    let capture = LogCapture::default();
    CAPTURE.with(|slot| {
        slot.replace(Some(capture.clone()));
    });
    (capture, CaptureGuard)
}

fn assert_no_proof_leak(dump: &str, header: &str) {
    assert!(
        !dump.contains(header),
        "must not log SIGN-IN-WITH-X header: {dump}"
    );
    assert!(
        !dump.contains("https://") && !dump.contains("http://"),
        "must not log URL: {dump}"
    );
}

#[tokio::test]
async fn parse_denied_logs_error_not_header() {
    let fac = Arc::new(FakeFacilitator::new());
    let store = InMemoryPaidAddressStore::new();
    let layer = middleware(Arc::clone(&fac), FlowScheme::authorization())
        .with_siwx(gate(store))
        .with_price_tag(eip155_tag())
        .unwrap();
    let header = "not-valid-base64!!!";
    let req = siwx_request(HeaderValue::from_static(header), "api.example.com");
    let (capture, _guard) = capture_logs();
    let response = call_layer(layer, OkInner, req).await;
    assert_eq!(
        response.status(),
        StatusCode::PAYMENT_REQUIRED,
        "parse fail must fail-open to 402"
    );
    assert_eq!(fac.verify_count(), 0, "parse fail must skip /verify");
    assert_eq!(
        capture.messages(),
        vec!["siwx parse denied".to_owned()],
        "parse fail logs once"
    );
    let dump = capture.dump();
    assert!(
        dump.contains("InvalidEncoding"),
        "parse deny must Debug the proof error: {dump}"
    );
    assert_no_proof_leak(&dump, header);
}

#[tokio::test]
async fn invalid_field_parse_denied_logs_error() {
    let fac = Arc::new(FakeFacilitator::new());
    let store = InMemoryPaidAddressStore::new();
    let layer = middleware(Arc::clone(&fac), FlowScheme::authorization())
        .with_siwx(gate(store))
        .with_price_tag(eip155_tag())
        .unwrap();
    let body = serde_json::json!({
        "domain": "api.example.com",
        "address": "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266",
        "uri": "https://api.example.com/paid",
        "version": "2",
        "chainId": "eip155:8453",
        "type": "eip191",
        "nonce": "a1b2c3d4e5f67890a1b2c3d4e5f67890",
        "issuedAt": "2024-01-15T10:30:00.000Z",
        "signature": "0xabc"
    });
    let encoded = Base64Bytes::encode(serde_json::to_vec(&body).unwrap());
    let header = encoded.to_string();
    let req = siwx_request(
        HeaderValue::from_bytes(encoded.as_ref()).unwrap(),
        "api.example.com",
    );
    let (capture, _guard) = capture_logs();
    let response = call_layer(layer, OkInner, req).await;
    assert_eq!(
        response.status(),
        StatusCode::PAYMENT_REQUIRED,
        "invalid version must fail-open to 402"
    );
    assert_eq!(
        capture.messages(),
        vec!["siwx parse denied".to_owned()],
        "version reject is a parse deny"
    );
    let dump = capture.dump();
    assert!(
        dump.contains("InvalidField"),
        "parse deny must Debug InvalidField: {dump}"
    );
    assert_no_proof_leak(&dump, &header);
}

#[tokio::test]
async fn verify_denied_logs_spec_code() {
    let fac = Arc::new(FakeFacilitator::new());
    let store = InMemoryPaidAddressStore::new();
    let layer = middleware(Arc::clone(&fac), FlowScheme::authorization())
        .with_auth_only(gate(store))
        .with_price_tag(eip155_tag())
        .unwrap();
    let challenge = call_layer(layer.clone(), OkInner, unpaid_request()).await;
    let mut info = decode_required(&challenge)["extensions"][SIWX_KEY]["info"].clone();
    info["address"] = "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266".into();
    info["chainId"] = "eip155:8453".into();
    info["type"] = "eip191".into();
    info["signature"] = "0x1111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111".into();
    let encoded = Base64Bytes::encode(serde_json::to_vec(&info).unwrap());
    let header = encoded.to_string();
    let req = siwx_request(
        HeaderValue::from_bytes(encoded.as_ref()).unwrap(),
        "api.example.com",
    );
    let (capture, _guard) = capture_logs();
    let response = call_layer(layer, OkInner, req).await;
    assert_eq!(
        response.status(),
        StatusCode::PAYMENT_REQUIRED,
        "bad signature must fail-open to 402"
    );
    assert_eq!(fac.verify_count(), 0, "bad signature must skip /verify");
    assert_eq!(
        capture.messages(),
        vec!["siwx denied".to_owned()],
        "post-parse verify fail logs once"
    );
    let dump = capture.dump();
    assert!(
        dump.contains("code=invalid_siwx_"),
        "deny must log spec code: {dump}"
    );
    assert_no_proof_leak(&dump, &header);
}

#[tokio::test]
async fn valid_unpaid_logs_unpaid() {
    let fac = Arc::new(FakeFacilitator::new());
    let store = InMemoryPaidAddressStore::new();
    let layer = middleware(Arc::clone(&fac), FlowScheme::authorization())
        .with_siwx(gate(store))
        .with_price_tag(eip155_tag())
        .unwrap();
    let challenge = call_layer(layer.clone(), OkInner, unpaid_request()).await;
    let info = decode_required(&challenge)["extensions"][SIWX_KEY]["info"].clone();
    let (_, header) = sign_challenge(&info).await;
    let header_str = header.to_str().unwrap().to_owned();
    let (capture, _guard) = capture_logs();
    let response = call_layer(layer, OkInner, siwx_request(header, "api.example.com")).await;
    assert_eq!(
        response.status(),
        StatusCode::PAYMENT_REQUIRED,
        "valid unpaid must fail-open to 402"
    );
    assert_eq!(fac.verify_count(), 0, "valid unpaid must skip /verify");
    assert_eq!(
        capture.messages(),
        vec!["siwx valid unpaid".to_owned()],
        "store miss after verify logs unpaid"
    );
    assert_no_proof_leak(&capture.dump(), &header_str);
}

#[tokio::test]
async fn nonce_replay_logs_replay() {
    let fac = Arc::new(FakeFacilitator::new());
    let store = InMemoryPaidAddressStore::new();
    let layer = middleware(Arc::clone(&fac), FlowScheme::authorization())
        .with_auth_only(gate(store))
        .with_price_tag(eip155_tag())
        .unwrap();
    let challenge = call_layer(layer.clone(), OkInner, unpaid_request()).await;
    let info = decode_required(&challenge)["extensions"][SIWX_KEY]["info"].clone();
    let (_, header) = sign_challenge(&info).await;
    let first = call_layer(
        layer.clone(),
        OkInner,
        siwx_request(header.clone(), "api.example.com"),
    )
    .await;
    assert_eq!(first.status(), StatusCode::OK, "first consume must grant");
    let header_str = header.to_str().unwrap().to_owned();
    let (capture, _guard) = capture_logs();
    let second = call_layer(layer, OkInner, siwx_request(header, "api.example.com")).await;
    assert_eq!(
        second.status(),
        StatusCode::PAYMENT_REQUIRED,
        "replay must fail-open to 402"
    );
    assert_eq!(fac.verify_count(), 0, "replay must skip /verify");
    assert_eq!(
        capture.messages(),
        vec!["siwx nonce replay".to_owned()],
        "second consume logs replay"
    );
    assert_no_proof_leak(&capture.dump(), &header_str);
}

#[tokio::test]
async fn grant_logs_granted() {
    let fac = Arc::new(FakeFacilitator::new());
    let store = InMemoryPaidAddressStore::new();
    let layer = middleware(Arc::clone(&fac), FlowScheme::authorization())
        .with_auth_only(gate(store))
        .with_price_tag(eip155_tag())
        .unwrap();
    let challenge = call_layer(layer.clone(), OkInner, unpaid_request()).await;
    let info = decode_required(&challenge)["extensions"][SIWX_KEY]["info"].clone();
    let (_, header) = sign_challenge(&info).await;
    let header_str = header.to_str().unwrap().to_owned();
    let (capture, _guard) = capture_logs();
    let response = call_layer(layer, OkInner, siwx_request(header, "api.example.com")).await;
    assert_eq!(response.status(), StatusCode::OK, "auth_only must grant");
    assert_eq!(fac.verify_count(), 0, "grant must skip /verify");
    assert_eq!(
        capture.messages(),
        vec!["siwx granted".to_owned()],
        "successful consume logs grant"
    );
    assert_no_proof_leak(&capture.dump(), &header_str);
}
