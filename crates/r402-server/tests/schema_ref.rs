#![allow(unused_crate_dependencies, reason = "sibling tests consume them")]
#![allow(
    clippy::tests_outside_test_module,
    reason = "integration test binaries put #[test] fns at file scope"
)]
#![allow(
    clippy::unwrap_used,
    clippy::manual_async_fn,
    clippy::items_after_statements,
    clippy::missing_const_for_fn,
    clippy::missing_assert_message,
    clippy::panic,
    clippy::unnecessary_literal_bound,
    reason = "idiomatic test-code patterns"
)]

//! Bazaar extension schema `$ref`/`$id` rejection on verify.

use std::collections::HashMap;
use std::future::Future;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use r402_facilitator::Facilitator;
use r402_protocol::error::{ErrorReason, FacilitatorError, VerificationError};
use r402_protocol::network::ChainIdPattern;
use r402_protocol::payment::{
    ExtensionEntry, Extensions, PaymentRequirements, SettleRequest, SettleResponse,
    SupportedResponse, VerifyRequest, VerifyResponse,
};
use r402_server::{
    PaymentFlowConfig, ResourceServer, SDK_DEFAULT_ASSET_TRANSFER_METHOD, SchemeNetworkServer,
    WirePaymentPayload,
};
use serde_json::json;

#[derive(Debug)]
struct CountingFacilitator {
    verifies: AtomicUsize,
}

impl Facilitator for CountingFacilitator {
    fn verify(
        &self,
        _request: VerifyRequest,
    ) -> impl Future<Output = Result<VerifyResponse, FacilitatorError>> + Send {
        self.verifies.fetch_add(1, Ordering::SeqCst);
        std::future::ready(Ok(VerifyResponse::valid("0xpayer")))
    }

    fn settle(
        &self,
        _request: SettleRequest,
    ) -> impl Future<Output = Result<SettleResponse, FacilitatorError>> + Send {
        std::future::ready(Ok(SettleResponse::Success {
            payer: Some("0xpayer".into()),
            transaction: "0xtx".into(),
            network: "eip155:1".into(),
            amount: None,
            extensions: Extensions::new(),
            extension_responses: Extensions::new(),
            extra: None,
        }))
    }

    fn supported(
        &self,
    ) -> impl Future<Output = Result<SupportedResponse, FacilitatorError>> + Send {
        std::future::ready(Ok(SupportedResponse::default()))
    }
}

#[derive(Debug)]
struct AuthScheme {
    flows: HashMap<String, PaymentFlowConfig>,
}

impl AuthScheme {
    fn new() -> Self {
        let mut flows = HashMap::new();
        flows.insert(
            SDK_DEFAULT_ASSET_TRANSFER_METHOD.into(),
            PaymentFlowConfig::authorization_only(),
        );
        Self { flows }
    }
}

impl SchemeNetworkServer for AuthScheme {
    fn scheme(&self) -> &str {
        "exact"
    }

    fn default_asset_transfer_method(&self) -> &str {
        SDK_DEFAULT_ASSET_TRANSFER_METHOD
    }

    fn payment_flows(&self) -> &HashMap<String, PaymentFlowConfig> {
        &self.flows
    }
}

fn sample_payload() -> WirePaymentPayload {
    let req = PaymentRequirements::new(
        "exact".into(),
        "eip155:1".parse().unwrap(),
        "1".into(),
        "0xa".into(),
        "0xb".into(),
        60,
    );
    WirePaymentPayload::new(req, json!({"k": 1}))
}

fn server(facilitator: Arc<CountingFacilitator>) -> ResourceServer {
    ResourceServer::new(facilitator)
        .with_scheme(ChainIdPattern::wildcard("eip155"), AuthScheme::new())
}

fn bazaar_schema(schema: serde_json::Value) -> ExtensionEntry {
    ExtensionEntry::with_schema(json!({"registered": true}), schema)
}

const EXTERNAL_REF_MSG: &str = "schema must not contain external $ref/$id references";

fn assert_invalid_format(err: &FacilitatorError) {
    match err {
        FacilitatorError::Verification(VerificationError::InvalidFormat(msg)) => {
            assert_eq!(msg, EXTERNAL_REF_MSG);
        }
        other => panic!("expected InvalidFormat, got {other:?}"),
    }
}

#[tokio::test]
async fn verify_payload_bazaar_external_ref_is_invalid_format() {
    let facilitator = Arc::new(CountingFacilitator {
        verifies: AtomicUsize::new(0),
    });
    let rs = server(Arc::clone(&facilitator));
    let mut payload = sample_payload();
    payload.extensions.insert(
        "bazaar",
        bazaar_schema(json!({"$ref": "http://127.0.0.1/attacker-schema.json"})),
    );
    let req = payload.accepted.clone();
    let err = rs.verify_payment(&payload, &req, None).await.unwrap_err();
    assert_invalid_format(&err);
    assert_eq!(
        err.as_payment_problem().unwrap().reason(),
        ErrorReason::InvalidPayload
    );
    assert_eq!(facilitator.verifies.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn verify_advertised_bazaar_external_ref_is_invalid_format() {
    let facilitator = Arc::new(CountingFacilitator {
        verifies: AtomicUsize::new(0),
    });
    let rs = server(Arc::clone(&facilitator));
    let payload = sample_payload();
    let mut advertised = Extensions::new();
    advertised.insert(
        "bazaar",
        bazaar_schema(json!({"$ref": "file:///etc/passwd"})),
    );
    let req = payload.accepted.clone();
    let err = rs
        .verify_payment(&payload, &req, Some(&advertised))
        .await
        .unwrap_err();
    assert_invalid_format(&err);
    assert_eq!(facilitator.verifies.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn verify_bazaar_nested_ref_is_invalid_format() {
    let rs = server(Arc::new(CountingFacilitator {
        verifies: AtomicUsize::new(0),
    }));
    let mut payload = sample_payload();
    payload.extensions.insert(
        "bazaar",
        bazaar_schema(json!({
            "properties": { "input": { "$ref": "http://evil.example/schema.json" } }
        })),
    );
    let req = payload.accepted.clone();
    let err = rs.verify_payment(&payload, &req, None).await.unwrap_err();
    assert_invalid_format(&err);
}

#[tokio::test]
async fn verify_bazaar_non_string_ref_is_invalid_format() {
    let rs = server(Arc::new(CountingFacilitator {
        verifies: AtomicUsize::new(0),
    }));
    let mut payload = sample_payload();
    payload
        .extensions
        .insert("bazaar", bazaar_schema(json!({"$ref": 1})));
    let req = payload.accepted.clone();
    let err = rs.verify_payment(&payload, &req, None).await.unwrap_err();
    assert_invalid_format(&err);
}

#[tokio::test]
async fn verify_bazaar_fragment_ref_is_ok() {
    let facilitator = Arc::new(CountingFacilitator {
        verifies: AtomicUsize::new(0),
    });
    let rs = server(Arc::clone(&facilitator));
    let mut payload = sample_payload();
    payload.extensions.insert(
        "bazaar",
        bazaar_schema(json!({"$ref": "#/definitions/root"})),
    );
    let mut advertised = Extensions::new();
    advertised.insert(
        "bazaar",
        bazaar_schema(json!({"$ref": "#/definitions/root"})),
    );
    let req = payload.accepted.clone();
    let out = rs
        .verify_payment(&payload, &req, Some(&advertised))
        .await
        .unwrap();
    assert!(out.response.is_valid());
    assert_eq!(facilitator.verifies.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn verify_bazaar_schema_url_is_ok() {
    let facilitator = Arc::new(CountingFacilitator {
        verifies: AtomicUsize::new(0),
    });
    let rs = server(Arc::clone(&facilitator));
    let mut payload = sample_payload();
    payload.extensions.insert(
        "bazaar",
        bazaar_schema(json!({
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "type": "object"
        })),
    );
    let req = payload.accepted.clone();
    let out = rs.verify_payment(&payload, &req, None).await.unwrap();
    assert!(out.response.is_valid());
    assert_eq!(facilitator.verifies.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn verify_bazaar_raw_entry_is_not_checked() {
    let facilitator = Arc::new(CountingFacilitator {
        verifies: AtomicUsize::new(0),
    });
    let rs = server(Arc::clone(&facilitator));
    let mut payload = sample_payload();
    payload.extensions.insert(
        "bazaar",
        ExtensionEntry::raw(json!({"$ref": "http://127.0.0.1/attacker-schema.json"})),
    );
    let req = payload.accepted.clone();
    let out = rs.verify_payment(&payload, &req, None).await.unwrap();
    assert!(out.response.is_valid());
    assert_eq!(facilitator.verifies.load(Ordering::SeqCst), 1);
}
