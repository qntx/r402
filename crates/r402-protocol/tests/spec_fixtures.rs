#![allow(unused_crate_dependencies, reason = "sibling tests consume them")]
#![allow(
    clippy::tests_outside_test_module,
    reason = "integration test binaries put #[test] fns at file scope"
)]
#![allow(
    clippy::indexing_slicing,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "idiomatic test-code patterns"
)]

//! Cross-SDK wire-format compatibility tests against `tests/fixtures/spec_v2`.

use std::fs;
use std::path::PathBuf;

use r402_protocol::payment::{
    PaymentPayload, PaymentRequired, SettleResponse, SupportedResponse, V2, VerifyResponse,
};

fn spec_v2_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/spec_v2")
}

fn fixture(name: &str) -> serde_json::Value {
    let path = spec_v2_dir().join(name);
    let bytes =
        fs::read(&path).unwrap_or_else(|err| panic!("missing fixture {}: {err}", path.display()));
    serde_json::from_slice(&bytes)
        .unwrap_or_else(|err| panic!("invalid JSON in fixture {}: {err}", path.display()))
}

fn assert_roundtrip<T>(value: &serde_json::Value)
where
    T: serde::Serialize + serde::de::DeserializeOwned,
{
    let parsed: T = serde_json::from_value(value.clone()).expect("fixture deserialises");
    let again = serde_json::to_value(&parsed).expect("re-serialises");
    assert_eq!(&again, value, "fixture round-trip diverged from input");
}

#[test]
fn payment_required_fixture_roundtrips() {
    assert_roundtrip::<PaymentRequired>(&fixture("payment_required.json"));
}

#[test]
fn payment_payload_eip3009_fixture_deserialises() {
    type Payload = PaymentPayload<serde_json::Value, serde_json::Value>;
    let json = fixture("payment_payload_eip3009.json");
    let parsed: Payload = serde_json::from_value(json.clone()).expect("fixture deserialises");
    assert_eq!(parsed.x402_version, V2);
    let again = serde_json::to_value(&parsed).expect("re-serialises");
    assert_eq!(again, json);
}

#[test]
fn verify_response_valid_fixture_roundtrips() {
    let json = fixture("verify_response_valid.json");
    let parsed: VerifyResponse =
        serde_json::from_value(json.clone()).expect("fixture deserialises");
    assert!(parsed.is_valid(), "fixture is the canonical Valid variant");
    let again = serde_json::to_value(&parsed).expect("re-serialises");
    assert_eq!(again, json);
}

#[test]
fn verify_response_invalid_fixture_roundtrips() {
    let json = fixture("verify_response_invalid.json");
    let parsed: VerifyResponse =
        serde_json::from_value(json.clone()).expect("fixture deserialises");
    assert!(
        !parsed.is_valid(),
        "fixture must deserialise into the Invalid variant"
    );
    let again = serde_json::to_value(&parsed).expect("re-serialises");
    assert_eq!(again, json);
}

#[test]
fn settle_response_success_fixture_roundtrips() {
    let json = fixture("settle_response_success.json");
    let parsed: SettleResponse =
        serde_json::from_value(json.clone()).expect("fixture deserialises");
    assert!(
        parsed.is_success(),
        "fixture is the canonical Success variant"
    );
    let again = serde_json::to_value(&parsed).expect("re-serialises");
    assert_eq!(again, json);
}

#[test]
fn settle_response_failure_fixture_roundtrips() {
    let json = fixture("settle_response_failure.json");
    let parsed: SettleResponse =
        serde_json::from_value(json.clone()).expect("fixture deserialises");
    assert!(!parsed.is_success());
    let again = serde_json::to_value(&parsed).expect("re-serialises");
    assert_eq!(again["transaction"], "");
    assert_eq!(again, json);
}

#[test]
fn supported_response_fixture_roundtrips() {
    assert_roundtrip::<SupportedResponse>(&fixture("supported_response.json"));
}

#[test]
fn every_fixture_has_a_test() {
    let path = spec_v2_dir();
    let entries: Vec<_> = fs::read_dir(&path)
        .expect("fixture dir exists")
        .filter_map(Result::ok)
        .filter(|e| e.path().extension().is_some_and(|ext| ext == "json"))
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    let expected: &[&str] = &[
        "payment_required.json",
        "payment_payload_eip3009.json",
        "verify_response_valid.json",
        "verify_response_invalid.json",
        "settle_response_success.json",
        "settle_response_failure.json",
        "supported_response.json",
    ];
    for fixture_name in expected {
        assert!(
            entries.iter().any(|e| e == fixture_name),
            "expected fixture `{fixture_name}` missing from tests/fixtures/spec_v2/",
        );
    }
    assert_eq!(
        entries.len(),
        expected.len(),
        "fixture directory has {entries:?} but tests reference only {expected:?}; \
         add a deserialisation test for the new fixture or remove the file",
    );
}
