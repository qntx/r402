//! EIP-1271/6492 feature: missing-chain stays EIP-191; 6492 without RPC.

#![allow(unused_crate_dependencies, reason = "sibling tests consume them")]
#![allow(
    clippy::tests_outside_test_module,
    reason = "integration test binaries put #[test] fns at file scope"
)]
#![allow(
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::unwrap_used,
    clippy::panic,
    reason = "idiomatic test-code patterns"
)]

use alloy_signer::Signer;
use alloy_signer_local::PrivateKeySigner;
use r402_extensions::siwx::{EvmVerifier, SiwxError, SiwxOrigin, SiwxProof};
use r402_protocol::payment::Base64Bytes;
use serde_json::json;
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

const FIXTURE_KEY: &str = "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80";

const EIP6492_MAGIC: [u8; 32] = [
    0x64, 0x92, 0x64, 0x92, 0x64, 0x92, 0x64, 0x92, 0x64, 0x92, 0x64, 0x92, 0x64, 0x92, 0x64, 0x92,
    0x64, 0x92, 0x64, 0x92, 0x64, 0x92, 0x64, 0x92, 0x64, 0x92, 0x64, 0x92, 0x64, 0x92, 0x64, 0x92,
];

fn origin() -> SiwxOrigin {
    SiwxOrigin::parse("https://api.example.com").unwrap()
}

async fn signed_evm_proof(origin: &SiwxOrigin, path: &str) -> SiwxProof {
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
    let header = Base64Bytes::encode(serde_json::to_vec(&body).unwrap()).to_string();
    let mut proof = SiwxProof::parse_header(&header).unwrap();
    let raw = proof.signing_message().unwrap();
    let sig = signer.sign_message(raw.as_bytes()).await.unwrap();
    proof.signature = format!("{sig}").into();
    proof
}

#[tokio::test]
async fn verify_at_rpc_map_missing_chain_stays_191() {
    let origin = origin();
    let proof = signed_evm_proof(&origin, "/premium-data").await;
    let evm = EvmVerifier::with_rpc_map([(1u64, "http://127.0.0.1:1")]);
    proof
        .verify_at(&origin, "/premium-data", OffsetDateTime::now_utc(), &evm)
        .await
        .unwrap();
}

#[tokio::test]
async fn eip6492_magic_without_rpc_is_malformed() {
    let origin = origin();
    let mut proof = signed_evm_proof(&origin, "/premium-data").await;
    proof.signature = format!("0x{}", hex::encode(EIP6492_MAGIC)).into();
    let err = proof
        .verify_at(
            &origin,
            "/premium-data",
            OffsetDateTime::now_utc(),
            &EvmVerifier::new(),
        )
        .await
        .unwrap_err();
    assert_eq!(err, SiwxError::MalformedSignature);
}
