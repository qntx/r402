//! SIWX origin binding, paid-address store, challenge, and proof parse.

#![allow(unused_crate_dependencies, reason = "sibling tests consume them")]
#![allow(
    clippy::tests_outside_test_module,
    reason = "integration test binaries put #[test] fns at file scope"
)]
#![allow(
    clippy::indexing_slicing,
    clippy::unwrap_used,
    clippy::panic,
    reason = "idiomatic test-code patterns"
)]

use r402_extensions::siwx::{
    InMemoryPaidAddressStore, PaidAddressStore, SIWX_KEY, SiwxChain, SiwxError, SiwxExtension,
    SiwxOrigin, SiwxOriginError, SiwxProof,
};
use r402_protocol::extension::Extension;
use r402_protocol::payment::Base64Bytes;
use serde_json::json;

#[test]
fn origin_requires_absolute_http_url() {
    assert!(matches!(
        SiwxOrigin::parse(""),
        Err(SiwxOriginError::Missing)
    ));
    assert!(matches!(
        SiwxOrigin::parse("api.example.com"),
        Err(SiwxOriginError::NotAbsolute)
    ));
    assert!(matches!(
        SiwxOrigin::parse("api.example.com:443"),
        Err(SiwxOriginError::NotAbsolute)
    ));
}

#[test]
fn origin_never_accepts_host_header_shape() {
    let err = SiwxOrigin::parse("api.example.com").unwrap_err();
    assert_eq!(err, SiwxOriginError::NotAbsolute);
}

#[test]
fn origin_parses_https_and_strips_path() {
    let origin = SiwxOrigin::parse("https://api.example.com/premium-data").unwrap();
    assert_eq!(origin.as_str(), "https://api.example.com");
    assert_eq!(origin.domain(), "api.example.com");
    assert_eq!(
        origin.uri("/premium-data").as_str(),
        "https://api.example.com/premium-data"
    );
}

#[test]
fn origin_strips_default_port_and_lowercases_host() {
    let origin = SiwxOrigin::parse("HTTPS://API.Example.COM:443/premium-data").unwrap();
    assert_eq!(origin.as_str(), "https://api.example.com");
    assert_eq!(origin.domain(), "api.example.com");
}

#[test]
fn origin_keeps_non_default_port() {
    let origin = SiwxOrigin::parse("https://api.example.com:8443").unwrap();
    assert_eq!(origin.as_str(), "https://api.example.com:8443");
    assert_eq!(origin.domain(), "api.example.com:8443");
}

#[test]
fn origin_rejects_userinfo() {
    assert!(matches!(
        SiwxOrigin::parse("https://user@api.example.com"),
        Err(SiwxOriginError::InvalidAuthority)
    ));
}

#[test]
fn store_key_is_configured_origin_plus_path() {
    let origin = SiwxOrigin::parse("https://api.example.com").unwrap();
    assert_eq!(
        origin.store_key("/premium-data").as_str(),
        "https://api.example.com/premium-data"
    );
}

#[test]
fn paid_store_records_only_success() {
    let store = InMemoryPaidAddressStore::new();
    let key = "https://api.example.com/premium-data";
    let addr = "0x857b06519E91e3A54538791bDbb0E22373e36b66";
    assert!(!store.contains(key, addr));
    store.record_success(key, addr);
    assert!(store.contains(key, addr));
    assert!(!store.contains(key, "0xother"));
    assert!(!store.contains("https://api.example.com/other", addr));
}

#[test]
fn paid_store_clone_shares_records() {
    let store = InMemoryPaidAddressStore::new();
    let clone = store.clone();
    let key = "https://api.example.com/premium-data";
    let addr = "0x857b06519E91e3A54538791bDbb0E22373e36b66";
    store.record_success(key, addr);
    assert!(clone.contains(key, addr));
}

#[test]
fn challenge_uses_configured_origin_not_host() {
    let origin = SiwxOrigin::parse("https://api.example.com").unwrap();
    let ext = SiwxExtension::new(origin)
        .with_chain(SiwxChain::eip191("eip155:8453"))
        .with_statement("Sign in to access premium data");
    assert_eq!(ext.id(), SIWX_KEY);
    let entry = ext
        .challenge(
            "/premium-data",
            "a1b2c3d4e5f67890a1b2c3d4e5f67890",
            "2024-01-15T10:30:00.000Z",
            "2024-01-15T10:35:00.000Z",
        )
        .unwrap();
    let value = entry.to_value();
    assert_eq!(value["info"]["domain"], "api.example.com");
    assert_eq!(value["info"]["uri"], "https://api.example.com/premium-data");
    assert_eq!(value["supportedChains"][0]["chainId"], "eip155:8453");
    assert_eq!(value["supportedChains"][0]["type"], "eip191");
}

#[test]
fn challenge_rejects_short_nonce() {
    let origin = SiwxOrigin::parse("https://api.example.com").unwrap();
    let ext = SiwxExtension::new(origin);
    let err = ext
        .challenge(
            "/x",
            "deadbeef",
            "2024-01-15T10:30:00.000Z",
            "2024-01-15T10:35:00.000Z",
        )
        .unwrap_err();
    assert_eq!(err, SiwxError::Nonce);
    assert_eq!(err.as_str(), "invalid_siwx_nonce");
}

#[test]
fn proof_parses_header_and_binds_origin() {
    let origin = SiwxOrigin::parse("https://api.example.com").unwrap();
    let body = json!({
        "domain": "api.example.com",
        "address": "0x857b06519E91e3A54538791bDbb0E22373e36b66",
        "uri": "https://api.example.com/premium-data",
        "version": "1",
        "chainId": "eip155:8453",
        "type": "eip191",
        "nonce": "a1b2c3d4e5f67890a1b2c3d4e5f67890",
        "issuedAt": "2024-01-15T10:30:00.000Z",
        "expirationTime": "2024-01-15T10:35:00.000Z",
        "signature": "0xabc"
    });
    let header = Base64Bytes::encode(serde_json::to_vec(&body).unwrap()).to_string();
    let proof = SiwxProof::parse_header(&header).unwrap();
    assert_eq!(proof.address, "0x857b06519E91e3A54538791bDbb0E22373e36b66");
    proof.bind_origin(&origin, "/premium-data").unwrap();
    let mismatch = proof.bind_origin(&origin, "/other").unwrap_err();
    assert_eq!(mismatch, SiwxError::UriMismatch);
}
