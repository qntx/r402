#![allow(unused_crate_dependencies, reason = "sibling tests consume them")]
#![allow(
    clippy::tests_outside_test_module,
    reason = "integration test binaries put #[test] fns at file scope"
)]
#![allow(
    clippy::indexing_slicing,
    clippy::shadow_unrelated,
    clippy::unwrap_used,
    reason = "idiomatic test-code patterns"
)]

//! End-to-end integration tests over the **public** r402-core API.
//!
//! Unit tests in `src/` can reach into `pub(crate)` surfaces; this test
//! crate is compiled as a separate binary that links r402-core through
//! its `pub` interface only. Anything tested here is part of the
//! external contract and a regression failing here is a semver-relevant
//! event.

use r402_core::ErrorReason;
use r402_core::wire::{
    ExtensionEntry, Extensions, PaymentRequired, PaymentRequirements, ResourceInfo, SettleResponse,
    SupportedPaymentKind, SupportedResponse, V2, VerifyResponse,
};

/// The 402 response body must round-trip through serde without losing any
/// of the optional pieces the spec allows the seller to attach.
#[test]
fn payment_required_full_envelope_roundtrip() {
    let resource = ResourceInfo::new("https://api.example.com/premium")
        .with_description("Premium dataset")
        .with_mime_type("application/json");
    let req = PaymentRequirements::new(
        "exact".into(),
        "eip155:8453".parse().unwrap(),
        "1000000".into(),
        "0x209693Bc6afc0C5328bA36FaF03C514EF312287C".into(),
        "0x036CbD53842c5426634e7929541eC2318f3dCF7e".into(),
        60,
    )
    .with_extra(serde_json::json!({"name": "USDC", "version": "2"}));
    let mut extensions = Extensions::new();
    extensions.insert(
        "bazaar",
        ExtensionEntry::info(serde_json::json!({"registered": true})),
    );

    let body = PaymentRequired::new(resource)
        .add_accept(req)
        .with_error("missing X-PAYMENT")
        .with_extensions(extensions);

    let json = serde_json::to_value(&body).expect("serialize");
    let back: PaymentRequired = serde_json::from_value(json.clone()).expect("deserialize");

    // PaymentRequired doesn't impl PartialEq because its `accepts` carry
    // serde_json::Value extras, so compare via canonical JSON instead.
    let again = serde_json::to_value(&back).unwrap();
    assert_eq!(json, again);
    assert_eq!(body.x402_version, V2);
    assert_eq!(body.accepts.len(), 1);
}

/// /supported responses must accept clients that omit `extensions`
/// (a wallet running an older spec build), and must round-trip clients
/// that include them.
#[test]
fn supported_response_minimal_and_full_roundtrip() {
    // Minimal: just one kind, no extensions, no signers.
    let minimal = SupportedResponse::new().with_kinds(vec![SupportedPaymentKind::new(
        2,
        "exact",
        "eip155:8453",
    )]);
    let json = serde_json::to_value(&minimal).unwrap();
    assert_eq!(json["kinds"][0]["scheme"], "exact");
    assert_eq!(json["extensions"], serde_json::json!([]));
    let back: SupportedResponse = serde_json::from_value(json).unwrap();
    assert_eq!(back, minimal);

    // Full: kinds + extensions + signers map.
    let full = SupportedResponse::new()
        .with_kinds(vec![
            SupportedPaymentKind::new(2, "exact", "eip155:8453"),
            SupportedPaymentKind::new(2, "exact", "solana:mainnet").with_extra(serde_json::json!({
                "feePayer": "CKPKJWNdJEqa81x7CkZ14BVPiY6y16Sxs7owznqtWYp5",
            })),
        ])
        .with_extensions(vec!["bazaar".into(), "payment-identifier".into()])
        .with_signers({
            let mut m = std::collections::HashMap::new();
            m.insert(
                "eip155:*".into(),
                vec!["0x1234567890abcdef1234567890abcdef12345678".into()],
            );
            m.insert(
                "solana:*".into(),
                vec!["CKPKJWNdJEqa81x7CkZ14BVPiY6y16Sxs7owznqtWYp5".into()],
            );
            m
        });
    let json = serde_json::to_value(&full).unwrap();
    let back: SupportedResponse = serde_json::from_value(json).unwrap();
    assert_eq!(back, full);

    // Wildcard signer lookup must hit the `eip155:*` entry for any
    // chain in the eip155 namespace.
    let chain: r402_core::chain::ChainId = "eip155:8453".parse().unwrap();
    let signers = back.signers_for_chain(&chain);
    assert_eq!(signers.len(), 1);
    assert!(signers[0].starts_with("0x"));
}

/// Spec §5.3 / Fix-2: `SettleResponse::Failure` MUST always serialise
/// `transaction: ""`, never omit it. Verified end-to-end through the
/// public type.
#[test]
fn settle_failure_emits_empty_transaction() {
    let failure = SettleResponse::Failure {
        reason: ErrorReason::DuplicateSettlement,
        message: Some("already processed".into()),
        payer: Some("0xpayer".into()),
        network: "eip155:8453".into(),
        extensions: Extensions::new(),
    };
    let json = serde_json::to_value(&failure).unwrap();
    assert_eq!(
        json["transaction"], "",
        "F-002 regression: empty transaction must be present, not omitted"
    );
    assert_eq!(json["errorReason"], "duplicate_settlement");
    let back: SettleResponse = serde_json::from_value(json).unwrap();
    assert_eq!(back, failure);
}

/// `VerifyResponse::Invalid` must serialise machine-readable error codes
/// that round-trip through the spec §9 catalogue and the `Custom`
/// catch-all without information loss.
#[test]
fn verify_response_invalid_preserves_unknown_reason() {
    let unknown_code = "vendor_specific_diagnostic_42";
    let resp = VerifyResponse::invalid(Some("0xabc".into()), ErrorReason::from_wire(unknown_code));
    let json = serde_json::to_value(&resp).unwrap();
    assert_eq!(json["isValid"], false);
    assert_eq!(json["invalidReason"], unknown_code);
    let back: VerifyResponse = serde_json::from_value(json).unwrap();
    assert_eq!(back, resp);
}

/// `deny_unknown_fields` (F-001) must reject typo'd top-level fields on
/// every wire envelope so client / facilitator typos surface as parse
/// errors rather than silently-defaulted fields.
#[test]
fn unknown_field_rejection_for_top_level_envelopes() {
    let with_typo = serde_json::json!({
        "url": "https://example.com",
        "descriptionTypo": "should reject",
    });
    assert!(serde_json::from_value::<ResourceInfo>(with_typo).is_err());

    let kind_typo = serde_json::json!({
        "x402Version": 2,
        "scheme": "exact",
        "network": "eip155:1",
        "extraTypo": null,
    });
    assert!(serde_json::from_value::<SupportedPaymentKind>(kind_typo).is_err());
}

/// Settlement-cache callers (every chain facilitator) use this public type.
/// A representative Solana-style key (base64 transaction payload) must
/// reserve once and then report a duplicate — the remaining API after the
/// `r402-svm::settlement_cache` compatibility module was removed.
#[cfg(feature = "cache")]
#[test]
fn settlement_cache_reserve_then_duplicate() {
    use r402_core::cache::{Duplicate, SettlementCache};

    let cache = SettlementCache::new();
    let key = "solana:mainnet:AQAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAABAAED";
    assert_eq!(cache.reserve(key), Duplicate::No);
    assert_eq!(cache.reserve(key), Duplicate::Yes);
    assert_eq!(cache.reserve("eip155:8453:0xabc"), Duplicate::No);
    assert_eq!(cache.reserve("eip155:8453:0xabc"), Duplicate::Yes);
}
