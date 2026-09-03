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

//! End-to-end tests over the public r402-protocol API.

use r402_protocol::payment::{
    ExtensionEntry, Extensions, PaymentRequired, PaymentRequirements, ResourceInfo, SettleResponse,
    SupportedPaymentKind, SupportedResponse, V2, VerifyResponse,
};
use r402_protocol::{ErrorReason, FacilitatorError, FacilitatorTransportKind};

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
        .with_error("missing Payment-Signature")
        .with_extensions(extensions);

    let json = serde_json::to_value(&body).expect("serialize");
    let back: PaymentRequired = serde_json::from_value(json.clone()).expect("deserialize");

    let again = serde_json::to_value(&back).unwrap();
    assert_eq!(json, again);
    assert_eq!(body.x402_version, V2);
    assert_eq!(body.accepts.len(), 1);
    assert_eq!(json["error"], "missing Payment-Signature");
}

#[test]
fn supported_response_minimal_and_full_roundtrip() {
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

    let chain: r402_protocol::ChainId = "eip155:8453".parse().unwrap();
    let signers = back.signers_for_chain(&chain);
    assert_eq!(signers.len(), 1);
    assert!(signers[0].starts_with("0x"));
}

#[test]
fn settle_failure_emits_empty_transaction() {
    let failure = SettleResponse::Failure {
        reason: ErrorReason::DuplicateSettlement,
        message: Some("already processed".into()),
        payer: Some("0xpayer".into()),
        transaction: compact_str::CompactString::default(),
        network: "eip155:8453".into(),
        extensions: Extensions::new(),
        extension_responses: Extensions::new(),
        extra: None,
    };
    let json = serde_json::to_value(&failure).unwrap();
    assert_eq!(
        json["transaction"], "",
        "empty transaction must be present, not omitted"
    );
    assert_eq!(json["errorReason"], "duplicate_settlement");
    assert!(json.get("extensionResponses").is_none());
    let back: SettleResponse = serde_json::from_value(json).unwrap();
    assert_eq!(back, failure);
}

#[test]
fn settle_failure_pending_roundtrips_non_empty_transaction() {
    let failure = SettleResponse::Failure {
        reason: ErrorReason::SettlementPending,
        message: Some("receipt wait timed out".into()),
        payer: Some("0xpayer".into()),
        transaction: "0xabc".into(),
        network: "eip155:8453".into(),
        extensions: Extensions::new(),
        extension_responses: Extensions::new(),
        extra: None,
    };
    let json = serde_json::to_value(&failure).unwrap();
    assert_eq!(json["errorReason"], "settlement_pending");
    assert_eq!(json["transaction"], "0xabc");
    let back: SettleResponse = serde_json::from_value(json).unwrap();
    assert_eq!(back, failure);
    assert!(back.is_retryable_settlement_pending());
}

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

#[test]
fn transport_error_is_public() {
    let err = FacilitatorError::transport(FacilitatorTransportKind::MalformedSuccessBody);
    assert!(err.is_transport());
    assert!(matches!(
        err,
        FacilitatorError::Transport {
            kind: FacilitatorTransportKind::MalformedSuccessBody
        }
    ));
}

#[test]
fn background_settle_metric_name() {
    assert_eq!(
        r402_protocol::metrics::BACKGROUND_SETTLE_TOTAL,
        "r402_background_settle_total"
    );
}

#[test]
fn schema_has_external_ref_is_crate_root() {
    use r402_protocol::schema_has_external_ref;

    assert!(schema_has_external_ref(&serde_json::json!({
        "$ref": "http://127.0.0.1/attacker-schema.json"
    })));
    assert!(schema_has_external_ref(&serde_json::json!({
        "$ref": "file:///etc/passwd"
    })));
    assert!(schema_has_external_ref(&serde_json::json!({
        "properties": { "input": { "$ref": "http://evil.example/schema.json" } }
    })));
    assert!(schema_has_external_ref(&serde_json::json!({ "$ref": 1 })));
    assert!(!schema_has_external_ref(&serde_json::json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema"
    })));
    assert!(!schema_has_external_ref(&serde_json::json!({
        "$ref": "#/definitions/root"
    })));
    assert!(!schema_has_external_ref(&serde_json::json!({ "$id": "#" })));
}
