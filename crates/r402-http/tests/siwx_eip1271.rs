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

//! SIWX EIP-1271/6492 gate: server RPC map, EOA when chain is missing.

use std::time::Duration;

use alloy_signer::Signer;
use alloy_signer_local::PrivateKeySigner;
use axum_core::body::Body;
use axum_core::extract::Request;
use r402_http::server::{
    GateHooks, InMemoryPaidAddressStore, ProtectedRequestOutcome, SIGN_IN_WITH_X, SiwxChain,
    SiwxGate, SiwxOrigin, SiwxProof,
};
use r402_protocol::payment::Base64Bytes;
use serde_json::json;
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

const FIXTURE_KEY: &str = "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80";

fn origin() -> SiwxOrigin {
    SiwxOrigin::parse("https://api.example.com").unwrap()
}

async fn signed_evm_header(origin: &SiwxOrigin, path: &str) -> String {
    let signer: PrivateKeySigner = FIXTURE_KEY.parse().unwrap();
    let address = format!("{}", signer.address());
    let issued = OffsetDateTime::now_utc().format(&Rfc3339).unwrap();
    let body = json!({
        "domain": origin.domain(),
        "address": address,
        "uri": origin.uri(path),
        "version": "1",
        "chainId": "eip155:8453",
        "type": "eip191",
        "nonce": "a1b2c3d4e5f67890a1b2c3d4e5f67890",
        "issuedAt": issued,
        "signature": "00"
    });
    let encoded = Base64Bytes::encode(serde_json::to_vec(&body).unwrap()).to_string();
    let mut proof = SiwxProof::parse_header(&encoded).unwrap();
    let raw = proof.signing_message().unwrap();
    let sig = signer.sign_message(raw.as_bytes()).await.unwrap();
    proof.signature = format!("{sig}").into();
    proof.encode_header().unwrap()
}

fn siwx_request(header: &str, path: &str) -> Request {
    Request::builder()
        .uri(path)
        .header(SIGN_IN_WITH_X, header)
        .body(Body::empty())
        .unwrap()
}

fn mapped_auth_only_gate() -> SiwxGate {
    SiwxGate::new(origin(), InMemoryPaidAddressStore::new())
        .with_chain(SiwxChain::eip191("eip155:8453"))
        .with_auth_only()
        .with_evm_rpc_map([(1u64, "http://127.0.0.1:1")], None)
}

#[tokio::test]
async fn gate_rpc_map_missing_chain_still_grants_eoa() {
    let gate = mapped_auth_only_gate();
    let header = signed_evm_header(&origin(), "/premium-data").await;
    let req = siwx_request(&header, "/premium-data");
    let outcome = gate.on_protected_request(&req).await;
    assert!(matches!(outcome, ProtectedRequestOutcome::GrantAccess));
}

#[tokio::test]
async fn try_grant_uses_gate_verifier_not_verify_helper() {
    let gate = mapped_auth_only_gate();
    let cloned = gate.clone();
    let header = signed_evm_header(&origin(), "/premium-data").await;
    let req = siwx_request(&header, "/premium-data");
    let outcome = cloned.on_protected_request(&req).await;
    assert!(matches!(outcome, ProtectedRequestOutcome::GrantAccess));
}

#[test]
fn with_evm_rpc_map_applies_timeout_in_one_call() {
    let _gate = SiwxGate::new(origin(), InMemoryPaidAddressStore::new())
        .with_evm_rpc_map([(1u64, "http://127.0.0.1:1")], Some(Duration::from_secs(1)));
}
