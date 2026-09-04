//! SIWX origin binding, paid-address store, challenge, and proof parse.

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

use std::future::Future;
use std::pin::Pin;

use alloy_signer::Signer;
use alloy_signer_local::PrivateKeySigner;
use compact_str::CompactString;
use r402_client::ClientExtension;
use r402_extensions::siwx::{
    DEFAULT_STATEMENT, InMemoryPaidAddressStore, PaidAddressStore, SIWX_HTTP_HEADER, SIWX_KEY,
    SiwxChain, SiwxClientExtension, SiwxError, SiwxExtension, SiwxOrigin, SiwxOriginError,
    SiwxProof, SiwxProofError, SiwxSigner,
};
use r402_protocol::extension::Extension;
use r402_protocol::payment::{Base64Bytes, ExtensionEntry, PaymentRequired, ResourceInfo};
use serde_json::{Value, json};
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

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
    let ext = SiwxExtension::new(origin).with_chain(SiwxChain::eip191("eip155:8453"));
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
    assert_eq!(value["info"]["statement"], DEFAULT_STATEMENT);
    assert_eq!(value["supportedChains"][0]["chainId"], "eip155:8453");
    assert_eq!(value["supportedChains"][0]["type"], "eip191");
}

#[test]
fn challenge_includes_default_statement() {
    let origin = SiwxOrigin::parse("https://api.example.com").unwrap();
    let value = SiwxExtension::new(origin)
        .challenge(
            "/premium-data",
            "a1b2c3d4e5f67890a1b2c3d4e5f67890",
            "2024-01-15T10:30:00.000Z",
            "2024-01-15T10:35:00.000Z",
        )
        .unwrap()
        .to_value();
    assert_eq!(value["info"]["statement"], DEFAULT_STATEMENT);
}

#[test]
fn with_statement_overrides_default() {
    let origin = SiwxOrigin::parse("https://api.example.com").unwrap();
    let value = SiwxExtension::new(origin)
        .with_statement("Sign in to access premium data")
        .challenge(
            "/premium-data",
            "a1b2c3d4e5f67890a1b2c3d4e5f67890",
            "2024-01-15T10:30:00.000Z",
            "2024-01-15T10:35:00.000Z",
        )
        .unwrap()
        .to_value();
    assert_eq!(value["info"]["statement"], "Sign in to access premium data");
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

#[test]
fn paid_store_evm_address_is_case_insensitive() {
    let store = InMemoryPaidAddressStore::new();
    let key = "https://api.example.com/premium-data";
    store.record_success(key, "0xAbCDEF");
    assert!(store.contains(key, "0xabcdef"));
}

#[test]
fn nonce_pair_records_and_rejects_reuse() {
    let store = InMemoryPaidAddressStore::new();
    assert!(!store.has_used_nonce("n1"));
    store.record_nonce("n1");
    assert!(store.has_used_nonce("n1"));
    assert!(!store.has_used_nonce("n2"));
}

#[test]
fn consume_nonce_inserts_once() {
    let store = InMemoryPaidAddressStore::new();
    assert!(store.consume_nonce("n1"));
    assert!(!store.consume_nonce("n1"));
    assert!(store.has_used_nonce("n1"));
    let clone = store.clone();
    assert!(!clone.consume_nonce("n1"));
    assert!(clone.consume_nonce("n2"));
    assert!(store.has_used_nonce("n2"));
}

#[test]
fn challenge_now_uses_32_hex_nonce() {
    let origin = SiwxOrigin::parse("https://api.example.com").unwrap();
    let ext = SiwxExtension::new(origin).with_chain(SiwxChain::eip191("eip155:8453"));
    let value = ext.challenge_now("/premium-data").unwrap().to_value();
    let nonce = value["info"]["nonce"].as_str().unwrap();
    assert_eq!(nonce.len(), 32);
    assert!(nonce.chars().all(|c| c.is_ascii_hexdigit()));
    assert_eq!(value["info"]["domain"], "api.example.com");
    assert_eq!(value["info"]["statement"], DEFAULT_STATEMENT);
}

fn sample_proof_at(issued_at: &str, expiration: Option<&str>) -> SiwxProof {
    let origin = SiwxOrigin::parse("https://api.example.com").unwrap();
    let mut body = json!({
        "domain": origin.domain(),
        "address": "0x857b06519E91e3A54538791bDbb0E22373e36b66",
        "uri": origin.uri("/premium-data"),
        "version": "1",
        "chainId": "eip155:8453",
        "type": "eip191",
        "nonce": "a1b2c3d4e5f67890a1b2c3d4e5f67890",
        "issuedAt": issued_at,
        "signature": "0xabc"
    });
    if let Some(exp) = expiration {
        body["expirationTime"] = json!(exp);
    }
    let header = Base64Bytes::encode(serde_json::to_vec(&body).unwrap()).to_string();
    SiwxProof::parse_header(&header).unwrap()
}

#[test]
fn signing_message_uses_eip4361_no_statement_blank_lines() {
    let proof = sample_proof_at("2024-01-15T10:30:00.000Z", None);
    let raw = proof.signing_message().unwrap();
    assert!(
        raw.contains("account:\n0x857b06519E91e3A54538791bDbb0E22373e36b66\n\n\nURI: "),
        "siwx 0.6 no-statement form is address\\n\\n\\nURI:, got {raw}"
    );
    assert!(
        raw.contains("Issued At: 2024-01-15T10:30:00.000Z"),
        "issuedAt original must survive formatter: {raw}"
    );
}

#[test]
fn validate_rejects_stale_issued_at() {
    let origin = SiwxOrigin::parse("https://api.example.com").unwrap();
    let proof = sample_proof_at("2020-01-01T00:00:00Z", None);
    let now = OffsetDateTime::parse("2020-01-01T00:10:00Z", &Rfc3339).unwrap();
    assert_eq!(
        proof
            .validate_at(&origin, "/premium-data", now)
            .unwrap_err(),
        SiwxError::IssuedAtTooOld
    );
}

#[test]
fn validate_rejects_future_issued_at() {
    let origin = SiwxOrigin::parse("https://api.example.com").unwrap();
    let proof = sample_proof_at("2020-01-01T00:10:00Z", None);
    let now = OffsetDateTime::parse("2020-01-01T00:00:00Z", &Rfc3339).unwrap();
    assert_eq!(
        proof
            .validate_at(&origin, "/premium-data", now)
            .unwrap_err(),
        SiwxError::IssuedAtInFuture
    );
}

#[test]
fn validate_rejects_expired() {
    let origin = SiwxOrigin::parse("https://api.example.com").unwrap();
    let proof = sample_proof_at("2020-01-01T00:00:00Z", Some("2020-01-01T00:01:00Z"));
    let now = OffsetDateTime::parse("2020-01-01T00:02:00Z", &Rfc3339).unwrap();
    assert_eq!(
        proof
            .validate_at(&origin, "/premium-data", now)
            .unwrap_err(),
        SiwxError::Expired
    );
}

#[test]
fn unsupported_chain_is_unsupported_chain() {
    let body = json!({
        "domain": "api.example.com",
        "address": "0x857b06519E91e3A54538791bDbb0E22373e36b66",
        "uri": "https://api.example.com/premium-data",
        "version": "1",
        "chainId": "bip122:000000000019d6689c085ae165831e93",
        "type": "eip191",
        "nonce": "a1b2c3d4e5f67890a1b2c3d4e5f67890",
        "issuedAt": "2024-01-15T10:30:00.000Z",
        "signature": "0xabc"
    });
    let header = Base64Bytes::encode(serde_json::to_vec(&body).unwrap()).to_string();
    let proof = SiwxProof::parse_header(&header).unwrap();
    assert_eq!(
        proof.signing_message().unwrap_err(),
        SiwxError::UnsupportedChain
    );
}

const FIXTURE_KEY: &str = "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80";

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
async fn evm_caip122_roundtrip() {
    let origin = SiwxOrigin::parse("https://api.example.com").unwrap();
    let proof = signed_evm_proof(&origin, "/premium-data").await;
    proof.verify(&origin, "/premium-data").await.unwrap();
}

#[tokio::test]
async fn evm_bad_signature_is_signature() {
    let origin = SiwxOrigin::parse("https://api.example.com").unwrap();
    let mut proof = signed_evm_proof(&origin, "/premium-data").await;
    proof.statement = Some("tampered".into());
    let err = proof.verify(&origin, "/premium-data").await.unwrap_err();
    assert_eq!(err, SiwxError::Signature);
}

#[tokio::test]
async fn solana_caip122_roundtrip() {
    use ed25519_dalek::{Signer as _, SigningKey};
    let origin = SiwxOrigin::parse("https://api.example.com").unwrap();
    let seed: [u8; 32] = std::array::from_fn(|i| u8::try_from(i).unwrap_or(0));
    let sk = SigningKey::from_bytes(&seed);
    let address = bs58::encode(sk.verifying_key().to_bytes()).into_string();
    let issued = OffsetDateTime::now_utc().format(&Rfc3339).unwrap();
    let body = json!({
        "domain": origin.domain(),
        "address": address,
        "uri": origin.uri("/premium-data"),
        "version": "1",
        "chainId": "solana:5eykt4UsFv8P8NJdTREpY1vzqKqZKvdp",
        "type": "ed25519",
        "nonce": "a1b2c3d4e5f67890a1b2c3d4e5f67890",
        "issuedAt": issued,
        "signature": "1"
    });
    let header = Base64Bytes::encode(serde_json::to_vec(&body).unwrap()).to_string();
    let mut proof = SiwxProof::parse_header(&header).unwrap();
    let raw = proof.signing_message().unwrap();
    let sig = sk.sign(raw.as_bytes());
    proof.signature = bs58::encode(sig.to_bytes()).into_string().into();
    proof.verify(&origin, "/premium-data").await.unwrap();
}

#[tokio::test]
async fn solana_malformed_signature() {
    let origin = SiwxOrigin::parse("https://api.example.com").unwrap();
    let issued = OffsetDateTime::now_utc().format(&Rfc3339).unwrap();
    let body = json!({
        "domain": origin.domain(),
        "address": "GwAF45zjfyGzUbd3i3hXxzGeuchzEZXwpRYHZM5912F1",
        "uri": origin.uri("/premium-data"),
        "version": "1",
        "chainId": "solana:5eykt4UsFv8P8NJdTREpY1vzqKqZKvdp",
        "type": "ed25519",
        "nonce": "a1b2c3d4e5f67890a1b2c3d4e5f67890",
        "issuedAt": issued,
        "signature": "!!!"
    });
    let header = Base64Bytes::encode(serde_json::to_vec(&body).unwrap()).to_string();
    let proof = SiwxProof::parse_header(&header).unwrap();
    assert_eq!(
        proof.verify(&origin, "/premium-data").await.unwrap_err(),
        SiwxError::MalformedSignature
    );
}

#[test]
fn solana_identity_address_is_malformed_signature() {
    let mut proof = unsigned_proof("solana:5eykt4UsFv8P8NJdTREpY1vzqKqZKvdp", "ed25519");
    proof.address = "11111111111111111111111111111111".into();
    assert_eq!(
        proof.signing_message().unwrap_err(),
        SiwxError::MalformedSignature
    );
}

#[test]
fn evm_non_eip55_address_is_malformed_signature() {
    let mut proof = unsigned_proof("eip155:8453", "eip191");
    proof.address = "0xd8da6bf26964af9d7eed9e03e53415d37aa96045".into();
    assert_eq!(
        proof.signing_message().unwrap_err(),
        SiwxError::MalformedSignature
    );
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

fn advertised_required(origin: &SiwxOrigin, path: &str) -> PaymentRequired {
    let entry = SiwxExtension::new(origin.clone())
        .with_chain(SiwxChain::eip191("eip155:8453"))
        .challenge_now(path)
        .unwrap();
    let mut required = PaymentRequired::new(ResourceInfo::new(origin.uri(path)));
    required.extensions.insert(SIWX_KEY, entry);
    required
}

#[tokio::test]
async fn client_extension_sets_sign_in_with_x_from_configured_origin() {
    let origin = SiwxOrigin::parse("https://api.example.com").unwrap();
    let signer: PrivateKeySigner = FIXTURE_KEY.parse().unwrap();
    let ext = SiwxClientExtension::new().with_signer(FixtureEvm(signer));
    let required = advertised_required(&origin, "/premium-data");
    let headers = ext
        .on_payment_required(&required, "https://api.example.com/premium-data")
        .await;
    let header = headers
        .get(SIWX_HTTP_HEADER)
        .expect("SIGN-IN-WITH-X")
        .to_str()
        .unwrap();
    let proof = SiwxProof::parse_header(header).unwrap();
    assert_eq!(proof.domain, "api.example.com");
    assert_eq!(proof.uri, "https://api.example.com/premium-data");
    proof.verify(&origin, "/premium-data").await.unwrap();
}

#[tokio::test]
async fn client_extension_does_not_enrich_payload() {
    let origin = SiwxOrigin::parse("https://api.example.com").unwrap();
    let signer: PrivateKeySigner = FIXTURE_KEY.parse().unwrap();
    let ext = SiwxClientExtension::new().with_signer(FixtureEvm(signer));
    let required = advertised_required(&origin, "/premium-data");
    let enriched = ext
        .enrich_payment_payload("c2lnbmVk", &required)
        .await
        .unwrap();
    assert_eq!(enriched, "c2lnbmVk");
}

#[tokio::test]
async fn client_extension_skips_host_shaped_request_url() {
    let origin = SiwxOrigin::parse("https://api.example.com").unwrap();
    let signer: PrivateKeySigner = FIXTURE_KEY.parse().unwrap();
    let ext = SiwxClientExtension::new().with_signer(FixtureEvm(signer));
    let required = advertised_required(&origin, "/premium-data");
    let headers = ext.on_payment_required(&required, "evil.example").await;
    assert!(headers.get(SIWX_HTTP_HEADER).is_none());
}

fn raw_challenge(
    resource_url: &str,
    domain: &str,
    uri: &str,
    resources: &[&str],
) -> PaymentRequired {
    let mut required = PaymentRequired::new(ResourceInfo::new(resource_url));
    required.extensions.insert(
        SIWX_KEY,
        ExtensionEntry::raw(json!({
            "info": {
                "domain": domain,
                "uri": uri,
                "version": "1",
                "nonce": "a1b2c3d4e5f67890a1b2c3d4e5f67890",
                "issuedAt": "2024-01-15T10:30:00.000Z",
                "resources": resources,
            },
            "supportedChains": [{"chainId": "eip155:8453", "type": "eip191"}]
        })),
    );
    required
}

fn evm_client() -> SiwxClientExtension {
    let signer: PrivateKeySigner = FIXTURE_KEY.parse().unwrap();
    SiwxClientExtension::new().with_signer(FixtureEvm(signer))
}

async fn signs_siwx(required: &PaymentRequired, request_url: &str) -> bool {
    evm_client()
        .on_payment_required(required, request_url)
        .await
        .get(SIWX_HTTP_HEADER)
        .is_some()
}

#[tokio::test]
async fn client_extension_binds_to_request_url_not_resource_url() {
    let matching = raw_challenge(
        "https://evil.example/paid",
        "api.example.com",
        "https://api.example.com/premium-data",
        &[],
    );
    assert!(
        signs_siwx(&matching, "https://api.example.com/premium-data").await,
        "challenge matching the 402 URL must sign even when resource.url is another origin"
    );

    let resource_only = raw_challenge(
        "https://evil.example/paid",
        "evil.example",
        "https://evil.example/paid",
        &[],
    );
    assert!(
        !signs_siwx(&resource_only, "https://api.example.com/premium-data").await,
        "challenge matching resource.url only must not sign"
    );
}

#[tokio::test]
async fn client_extension_skips_domain_mismatch() {
    let required = raw_challenge(
        "https://api.example.com/premium-data",
        "evil.example",
        "https://api.example.com/premium-data",
        &[],
    );
    assert!(!signs_siwx(&required, "https://api.example.com/premium-data").await);
}

#[tokio::test]
async fn client_extension_skips_uri_origin_mismatch() {
    let required = raw_challenge(
        "https://api.example.com/premium-data",
        "api.example.com",
        "https://evil.example/paid",
        &[],
    );
    assert!(!signs_siwx(&required, "https://api.example.com/premium-data").await);
}

#[tokio::test]
async fn client_extension_allows_cross_origin_resources() {
    let required = raw_challenge(
        "https://evil.example/paid",
        "api.example.com",
        "https://api.example.com/other",
        &["https://cdn.other.example/asset.png"],
    );
    assert!(
        signs_siwx(&required, "https://api.example.com/premium-data").await,
        "resources may be cross-origin; uri path need not match the 402 path"
    );
}

#[tokio::test]
async fn client_extension_skips_incompatible_signer() {
    let origin = SiwxOrigin::parse("https://api.example.com").unwrap();
    let signer: PrivateKeySigner = FIXTURE_KEY.parse().unwrap();
    let ext = SiwxClientExtension::new().with_signer(FixtureEvm(signer));
    let entry = SiwxExtension::new(origin.clone())
        .with_chain(SiwxChain::ed25519(
            "solana:5eykt4UsFv8P8NJdTREpY1vzqKqZKvdp",
        ))
        .challenge_now("/premium-data")
        .unwrap();
    let mut required = PaymentRequired::new(ResourceInfo::new(origin.uri("/premium-data")));
    required.extensions.insert(SIWX_KEY, entry);
    let headers = ext
        .on_payment_required(&required, "https://api.example.com/premium-data")
        .await;
    assert!(headers.get(SIWX_HTTP_HEADER).is_none());
}

#[test]
fn proof_encode_header_roundtrips() {
    let origin = SiwxOrigin::parse("https://api.example.com").unwrap();
    let body = json!({
        "domain": origin.domain(),
        "address": "0x857b06519E91e3A54538791bDbb0E22373e36b66",
        "uri": origin.uri("/premium-data"),
        "version": "1",
        "chainId": "eip155:8453",
        "type": "eip191",
        "nonce": "a1b2c3d4e5f67890a1b2c3d4e5f67890",
        "issuedAt": "2024-01-15T10:30:00.000Z",
        "signature": "0xabc"
    });
    let header = Base64Bytes::encode(serde_json::to_vec(&body).unwrap()).to_string();
    let proof = SiwxProof::parse_header(&header).unwrap();
    let encoded = proof.encode_header().unwrap();
    let again = SiwxProof::parse_header(&encoded).unwrap();
    assert_eq!(again, proof);
}

fn unsigned_proof(chain_id: &str, signature_type: &str) -> SiwxProof {
    let body = json!({
        "domain": "api.example.com",
        "address": "0x857b06519E91e3A54538791bDbb0E22373e36b66",
        "uri": "https://api.example.com/premium-data",
        "version": "1",
        "chainId": chain_id,
        "type": signature_type,
        "nonce": "a1b2c3d4e5f67890a1b2c3d4e5f67890",
        "issuedAt": "2024-01-15T10:30:00.000Z",
        "signature": "0xabc"
    });
    let header = Base64Bytes::encode(serde_json::to_vec(&body).unwrap()).to_string();
    SiwxProof::parse_header(&header).unwrap()
}

fn decode_proof_json(encoded: &str) -> Value {
    let decoded = Base64Bytes(encoded.as_bytes().to_vec()).decode().unwrap();
    serde_json::from_slice(&decoded).unwrap()
}

#[test]
fn parse_rejects_version_not_one() {
    let body = json!({
        "domain": "api.example.com",
        "address": "0x857b06519E91e3A54538791bDbb0E22373e36b66",
        "uri": "https://api.example.com/premium-data",
        "version": "2",
        "chainId": "eip155:8453",
        "type": "eip191",
        "nonce": "a1b2c3d4e5f67890a1b2c3d4e5f67890",
        "issuedAt": "2024-01-15T10:30:00.000Z",
        "signature": "0xabc"
    });
    let header = Base64Bytes::encode(serde_json::to_vec(&body).unwrap()).to_string();
    assert_eq!(
        SiwxProof::parse_header(&header).unwrap_err(),
        SiwxProofError::InvalidField
    );
}

#[test]
fn signing_message_rejects_version_not_one() {
    let mut proof = unsigned_proof("eip155:8453", "eip191");
    proof.version = "2".into();
    assert_eq!(proof.signing_message().unwrap_err(), SiwxError::Signature);
}

#[test]
fn validate_at_does_not_check_version_or_type() {
    let origin = SiwxOrigin::parse("https://api.example.com").unwrap();
    let mut proof = sample_proof_at("2020-01-01T00:00:00Z", None);
    proof.version = "2".into();
    proof.signature_type = "ed25519".into();
    let now = OffsetDateTime::parse("2020-01-01T00:01:00Z", &Rfc3339).unwrap();
    proof.validate_at(&origin, "/premium-data", now).unwrap();
}

#[test]
fn type_eip191_with_solana_chain_is_unsupported() {
    let proof = unsigned_proof("solana:5eykt4UsFv8P8NJdTREpY1vzqKqZKvdp", "eip191");
    assert_eq!(
        proof.signing_message().unwrap_err(),
        SiwxError::UnsupportedChain
    );
}

#[test]
fn type_ed25519_with_eip155_chain_is_unsupported() {
    let proof = unsigned_proof("eip155:8453", "ed25519");
    assert_eq!(
        proof.signing_message().unwrap_err(),
        SiwxError::UnsupportedChain
    );
}

#[test]
fn eip155_leading_zero_chain_id_is_chain_id() {
    let proof = unsigned_proof("eip155:01", "eip191");
    assert_eq!(proof.signing_message().unwrap_err(), SiwxError::ChainId);
}

#[test]
fn eip155_zero_chain_id_is_chain_id() {
    let proof = unsigned_proof("eip155:0", "eip191");
    assert_eq!(proof.signing_message().unwrap_err(), SiwxError::ChainId);
}

#[test]
fn solana_bad_charset_chain_id() {
    let proof = unsigned_proof("solana:main.net", "ed25519");
    assert_eq!(proof.signing_message().unwrap_err(), SiwxError::ChainId);
}

#[test]
fn bind_origin_rejects_origin_only_uri() {
    let origin = SiwxOrigin::parse("https://api.example.com").unwrap();
    let mut proof = sample_proof_at("2024-01-15T10:30:00.000Z", None);
    proof.uri = "https://api.example.com/".into();
    assert_eq!(
        proof.bind_origin(&origin, "/premium-data").unwrap_err(),
        SiwxError::UriMismatch
    );
}

#[test]
fn signature_scheme_roundtrips() {
    let mut body = json!({
        "domain": "api.example.com",
        "address": "0x857b06519E91e3A54538791bDbb0E22373e36b66",
        "uri": "https://api.example.com/premium-data",
        "version": "1",
        "chainId": "eip155:8453",
        "type": "eip191",
        "signatureScheme": "eip6492",
        "nonce": "a1b2c3d4e5f67890a1b2c3d4e5f67890",
        "issuedAt": "2024-01-15T10:30:00.000Z",
        "signature": "0xabc"
    });
    let header = Base64Bytes::encode(serde_json::to_vec(&body).unwrap()).to_string();
    let proof = SiwxProof::parse_header(&header).unwrap();
    assert_eq!(proof.signature_scheme.as_deref(), Some("eip6492"));
    let encoded = proof.encode_header().unwrap();
    let again = SiwxProof::parse_header(&encoded).unwrap();
    assert_eq!(again.signature_scheme.as_deref(), Some("eip6492"));
    assert_eq!(decode_proof_json(&encoded)["signatureScheme"], "eip6492");

    body.as_object_mut().unwrap().remove("signatureScheme");
    let none_header = Base64Bytes::encode(serde_json::to_vec(&body).unwrap()).to_string();
    let none = SiwxProof::parse_header(&none_header).unwrap();
    assert!(none.signature_scheme.is_none());
    let encoded_none = none.encode_header().unwrap();
    assert!(
        decode_proof_json(&encoded_none)
            .get("signatureScheme")
            .is_none()
    );
}

#[tokio::test]
async fn signature_scheme_is_ignored_for_eoa_verify() {
    let origin = SiwxOrigin::parse("https://api.example.com").unwrap();
    let mut proof = signed_evm_proof(&origin, "/premium-data").await;
    proof.signature_scheme = Some("eip6492".into());
    proof.verify(&origin, "/premium-data").await.unwrap();
}

#[tokio::test]
async fn proof_from_info_copies_chain_signature_scheme() {
    let origin = SiwxOrigin::parse("https://api.example.com").unwrap();
    let signer: PrivateKeySigner = FIXTURE_KEY.parse().unwrap();
    let ext = SiwxClientExtension::new().with_signer(FixtureEvm(signer));
    let mut required = PaymentRequired::new(ResourceInfo::new(origin.uri("/premium-data")));
    required.extensions.insert(
        SIWX_KEY,
        ExtensionEntry::raw(json!({
            "info": {
                "domain": "api.example.com",
                "uri": "https://api.example.com/premium-data",
                "version": "1",
                "nonce": "a1b2c3d4e5f67890a1b2c3d4e5f67890",
                "issuedAt": "2024-01-15T10:30:00.000Z"
            },
            "supportedChains": [{
                "chainId": "eip155:8453",
                "type": "eip191",
                "signatureScheme": "eip6492"
            }]
        })),
    );
    let headers = ext
        .on_payment_required(&required, "https://api.example.com/premium-data")
        .await;
    let header = headers
        .get(SIWX_HTTP_HEADER)
        .expect("SIGN-IN-WITH-X")
        .to_str()
        .unwrap();
    let proof = SiwxProof::parse_header(header).unwrap();
    assert_eq!(proof.signature_scheme.as_deref(), Some("eip6492"));
}
