// Test binaries pull all dev-dependencies of the crate, so anything
// only used by sibling test files (`proptest`, `tokio`) shows up as
// unused here.
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

//! Cross-SDK wire-format compatibility tests.
//!
//! Each fixture under `tests/fixtures/spec_v2/` is a verbatim JSON
//! example pulled from
//! [`3rdparty/x402/specs/x402-specification-v2.md`]. r402 must:
//!
//! 1. **Deserialize** every fixture into the corresponding wire type
//!    without losing fields.
//! 2. **Re-serialize** to a JSON value semantically equivalent to the
//!    fixture (field order and whitespace excluded).
//!
//! A fixture failing here means the spec moved and r402 lags behind
//! (or — worse — that another SDK speaking the same spec would reject
//! r402's output). Either way it's a P0 cross-SDK compatibility break.
//!
//! The fixtures double as living documentation: any developer can read
//! them to understand what r402 is expected to interoperate with.

use std::fs;
use std::path::PathBuf;

use r402_core::wire::{PaymentRequired, SettleResponse, SupportedResponse, V2, VerifyResponse};

/// Reads a fixture file into a parsed [`serde_json::Value`].
fn fixture(name: &str) -> serde_json::Value {
    let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    path.push("tests/fixtures/spec_v2");
    path.push(name);
    let bytes =
        fs::read(&path).unwrap_or_else(|err| panic!("missing fixture {}: {err}", path.display()));
    serde_json::from_slice(&bytes)
        .unwrap_or_else(|err| panic!("invalid JSON in fixture {}: {err}", path.display()))
}

/// Asserts that `value` deserialises into `T`, then re-serialises into
/// a JSON value semantically equal to `value`.
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
    // Use the type-erased PaymentPayload to keep the fixture chain-agnostic.
    type Payload = r402_core::wire::PaymentPayload<serde_json::Value, serde_json::Value>;
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
    // The fixture omits `extensions`; re-serialisation also omits it
    // (skip_serializing_if), so the canonical comparison holds.
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

/// F-002 cross-SDK regression: spec §5.3.2 requires
/// `transaction: ""` in the settlement response on failure. r402 MUST
/// emit that exact field both ways.
#[test]
fn settle_response_failure_fixture_roundtrips() {
    let json = fixture("settle_response_failure.json");
    let parsed: SettleResponse =
        serde_json::from_value(json.clone()).expect("fixture deserialises");
    assert!(!parsed.is_success());
    let again = serde_json::to_value(&parsed).expect("re-serialises");
    // F-002 lock-in: every failure response carries `transaction: ""`.
    assert_eq!(again["transaction"], "");
    assert_eq!(again, json);
}

#[test]
fn supported_response_fixture_roundtrips() {
    assert_roundtrip::<SupportedResponse>(&fixture("supported_response.json"));
}

/// Sanity check: every fixture file under `tests/fixtures/spec_v2/`
/// is reachable from a test above. A new fixture committed without a
/// matching test is a failure mode we want to fail loudly.
#[test]
fn every_fixture_has_a_test() {
    let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    path.push("tests/fixtures/spec_v2");
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
