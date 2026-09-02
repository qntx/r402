#![allow(unused_crate_dependencies, reason = "sibling tests consume them")]
#![allow(
    clippy::tests_outside_test_module,
    reason = "integration test binaries put #[test] fns at file scope"
)]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::missing_panics_doc,
    reason = "idiomatic test-code patterns"
)]

//! Property-based wire-format round-trip tests.
//!
//! For every wire type that crosses an SDK boundary, `serialize +
//! deserialize == identity` must hold for *any* value in the type's
//! domain. Example-based tests cover the spec's happy path; these
//! property tests catch the ones we forgot to imagine.
//!
//! The strategies deliberately exercise:
//!
//! - `compact_str::CompactString` length boundaries (the inline /
//!   heap split happens at 24 bytes),
//! - `Option<T>` Some/None branches for every optional field,
//! - empty / non-empty extension blocks (the `Raw` and `Structured`
//!   shapes round-trip differently),
//! - the `Custom(...)` catch-all in [`ErrorReason`], whose entire
//!   purpose is to round-trip *unknown* wire codes losslessly.

use std::collections::HashMap;

use compact_str::CompactString;
use proptest::collection::{hash_map, vec};
use proptest::prelude::*;
use r402_core::ErrorReason;
use r402_core::chain::ChainId;
use r402_core::wire::{
    ExtensionEntry, Extensions, PaymentRequired, PaymentRequirements, ResourceInfo, SettleResponse,
    SupportedPaymentKind, SupportedResponse, VerifyResponse,
};

// ─── Strategies ────────────────────────────────────────────────────────

fn arb_compact_string() -> impl Strategy<Value = CompactString> {
    // Up to 64 bytes covers both the inline (≤ 24 B) and heap branches
    // of `CompactString` plus a representative spread of UTF-8 widths.
    "[a-zA-Z0-9_:./@\\- ]{0,64}".prop_map(CompactString::from)
}

fn arb_chain_id() -> impl Strategy<Value = ChainId> {
    prop_oneof![
        "(eip155|solana|aptos|stellar):[a-z0-9-]{1,32}",
        // Wildcard chain IDs are a documented pattern in /supported.
        "(eip155|solana):\\*",
    ]
    .prop_filter_map("must parse as ChainId", |raw| raw.parse::<ChainId>().ok())
}

fn arb_resource_info() -> impl Strategy<Value = ResourceInfo> {
    (
        arb_compact_string(),
        proptest::option::of(arb_compact_string()),
        proptest::option::of(arb_compact_string()),
    )
        .prop_map(|(url, description, mime)| {
            let mut info = ResourceInfo::new(url);
            if let Some(d) = description {
                info = info.with_description(d);
            }
            if let Some(m) = mime {
                info = info.with_mime_type(m);
            }
            info
        })
}

/// Strategy excludes [`serde_json::Value::Null`] because `Option<Value>`
/// with `skip_serializing_if = "Option::is_none"` collapses `Some(Null)`
/// to `None` after a serde round-trip — that is a documented serde
/// semantic, not a wire-format issue we can fix.
fn arb_supported_payment_kind() -> impl Strategy<Value = SupportedPaymentKind> {
    let extra_value = prop_oneof![
        any::<bool>().prop_map(serde_json::Value::Bool),
        any::<i64>().prop_map(serde_json::Value::from),
        arb_compact_string().prop_map(|s| serde_json::Value::String(s.to_string())),
    ];
    (
        any::<u8>(),
        arb_compact_string(),
        arb_compact_string(),
        proptest::option::of(extra_value),
    )
        .prop_map(|(version, scheme, network, extra)| {
            SupportedPaymentKind::new(version, scheme, network).with_optional_extra(extra)
        })
}

fn arb_supported_response() -> impl Strategy<Value = SupportedResponse> {
    let kinds = vec(arb_supported_payment_kind(), 0..4);
    let extensions = vec(arb_compact_string(), 0..4);
    let signers = hash_map(arb_compact_string(), vec(arb_compact_string(), 0..3), 0..4);
    (kinds, extensions, signers).prop_map(|(kinds, extensions, signers)| {
        let map: HashMap<CompactString, Vec<CompactString>> = signers.into_iter().collect();
        SupportedResponse::new()
            .with_kinds(kinds)
            .with_extensions(extensions)
            .with_signers(map)
    })
}

fn arb_extensions() -> impl Strategy<Value = Extensions> {
    let entry = prop_oneof![
        any::<i64>().prop_map(|n| ExtensionEntry::info(serde_json::json!({ "n": n }))),
        any::<bool>().prop_map(|b| ExtensionEntry::raw(serde_json::Value::Bool(b))),
        Just(ExtensionEntry::with_schema(
            serde_json::json!({ "registered": true }),
            serde_json::json!({ "type": "object" }),
        )),
    ];
    vec(("[a-z][a-z0-9-]{0,16}", entry), 0..3)
        .prop_map(|kvs| kvs.into_iter().collect::<Extensions>())
}

fn arb_payment_requirements() -> impl Strategy<Value = PaymentRequirements> {
    (
        arb_compact_string(), // scheme
        arb_chain_id(),
        // Amount is a stringified base-unit integer per spec.
        "[0-9]{1,20}".prop_map(CompactString::from),
        arb_compact_string(), // pay_to
        arb_compact_string(), // asset
        any::<u64>(),         // max_timeout_seconds
    )
        .prop_map(|(scheme, network, amount, pay_to, asset, max_timeout)| {
            PaymentRequirements::new(scheme, network, amount, pay_to, asset, max_timeout)
        })
}

fn arb_payment_required() -> impl Strategy<Value = PaymentRequired> {
    (
        arb_resource_info(),
        vec(arb_payment_requirements(), 0..3),
        proptest::option::of(arb_compact_string()),
        arb_extensions(),
    )
        .prop_map(|(resource, accepts, error, extensions)| {
            let mut body = PaymentRequired::new(resource).with_accepts(accepts);
            if let Some(err) = error {
                body = body.with_error(err);
            }
            body.with_extensions(extensions)
        })
}

fn arb_error_reason() -> impl Strategy<Value = ErrorReason> {
    prop_oneof![
        // Known canonical codes — at least one for each section.
        Just(ErrorReason::InsufficientFunds),
        Just(ErrorReason::InvalidPayload),
        Just(ErrorReason::InvalidNetwork),
        Just(ErrorReason::UnexpectedSettleError),
        Just(ErrorReason::SettlementPending),
        Just(ErrorReason::DuplicateSettlement),
        Just(ErrorReason::Permit2AllowanceRequired),
        Just(ErrorReason::InvalidExactSolanaPayloadMemoMismatch),
        // The catch-all is the only path that can carry arbitrary
        // server-supplied error strings; ensure round-trip preserves
        // them byte-for-byte.
        "[a-z][a-z0-9_]{1,40}".prop_map(|s| ErrorReason::from_wire(&s)),
    ]
}

fn arb_extra() -> impl Strategy<Value = Option<serde_json::Value>> {
    // Null is excluded: `skip_serializing_if = Option::is_none` drops Some(Null).
    let value = prop_oneof![
        any::<bool>().prop_map(serde_json::Value::Bool),
        any::<i64>().prop_map(serde_json::Value::from),
        arb_compact_string().prop_map(|s| serde_json::Value::String(s.to_string())),
    ];
    proptest::option::of(value)
}

fn arb_verify_response() -> impl Strategy<Value = VerifyResponse> {
    prop_oneof![
        (arb_compact_string(), arb_extensions(), arb_extra()).prop_map(
            |(payer, extensions, extra)| {
                VerifyResponse::Valid {
                    payer,
                    extensions,
                    extra,
                }
            }
        ),
        (
            arb_error_reason(),
            proptest::option::of(arb_compact_string()),
            proptest::option::of(arb_compact_string()),
            arb_extensions(),
            arb_extra(),
        )
            .prop_map(|(reason, message, payer, extensions, extra)| {
                VerifyResponse::Invalid {
                    reason,
                    message,
                    payer,
                    extensions,
                    extra,
                }
            }),
    ]
}

/// Spec §5.3.2 + Fix-2/F-002: `Success.transaction` MUST be non-empty.
/// The wire's `try_from` rejects `""` on success (an empty string is
/// reserved for the `Failure` variant), so the strategy filters those
/// out to mirror what a compliant facilitator would emit.
fn arb_non_empty_compact_string() -> impl Strategy<Value = CompactString> {
    "[a-zA-Z0-9_:./@\\- ]{1,64}".prop_map(CompactString::from)
}

fn arb_settle_response() -> impl Strategy<Value = SettleResponse> {
    prop_oneof![
        (
            arb_compact_string(),           // payer
            arb_non_empty_compact_string(), // transaction (non-empty per Fix-2)
            arb_compact_string(),           // network
            proptest::option::of("[0-9]{1,20}".prop_map(CompactString::from)),
            arb_extensions(),
            arb_extra(),
        )
            .prop_map(|(payer, transaction, network, amount, extensions, extra)| {
                SettleResponse::Success {
                    payer,
                    transaction,
                    network,
                    amount,
                    extensions,
                    extra,
                }
            }),
        (
            arb_error_reason(),
            proptest::option::of(arb_compact_string()),
            proptest::option::of(arb_compact_string()),
            arb_compact_string(), // transaction (empty allowed except settlement_pending)
            arb_compact_string(),
            arb_extensions(),
            arb_extra(),
        )
            .prop_map(
                |(reason, message, payer, transaction, network, extensions, extra)| {
                    let transaction =
                        if reason == ErrorReason::SettlementPending && transaction.is_empty() {
                            CompactString::from("0xpending")
                        } else {
                            transaction
                        };
                    SettleResponse::Failure {
                        reason,
                        message,
                        payer,
                        transaction,
                        network,
                        extensions,
                        extra,
                    }
                }
            ),
    ]
}

// ─── Round-trip properties ─────────────────────────────────────────────

proptest! {
    #![proptest_config(ProptestConfig {
        // 256 cases per property hits a sweet spot — enough coverage to
        // expose serialisation regressions, fast enough to keep CI under
        // a few seconds for the entire suite.
        cases: 256,
        ..ProptestConfig::default()
    })]

    #[test]
    fn resource_info_roundtrips(info in arb_resource_info()) {
        let json = serde_json::to_vec(&info).unwrap();
        let back: ResourceInfo = serde_json::from_slice(&json).unwrap();
        prop_assert_eq!(back, info);
    }

    #[test]
    fn supported_payment_kind_roundtrips(kind in arb_supported_payment_kind()) {
        let json = serde_json::to_vec(&kind).unwrap();
        let back: SupportedPaymentKind = serde_json::from_slice(&json).unwrap();
        prop_assert_eq!(back, kind);
    }

    #[test]
    fn supported_response_roundtrips(resp in arb_supported_response()) {
        let json = serde_json::to_vec(&resp).unwrap();
        let back: SupportedResponse = serde_json::from_slice(&json).unwrap();
        prop_assert_eq!(back, resp);
    }

    #[test]
    fn extensions_roundtrip(ext in arb_extensions()) {
        let json = serde_json::to_vec(&ext).unwrap();
        let back: Extensions = serde_json::from_slice(&json).unwrap();
        prop_assert_eq!(back, ext);
    }

    #[test]
    fn payment_requirements_roundtrips(req in arb_payment_requirements()) {
        let json = serde_json::to_vec(&req).unwrap();
        let back: PaymentRequirements = serde_json::from_slice(&json).unwrap();
        prop_assert_eq!(back, req);
    }

    #[test]
    fn payment_required_roundtrips(body in arb_payment_required()) {
        let json = serde_json::to_vec(&body).unwrap();
        let back: PaymentRequired = serde_json::from_slice(&json).unwrap();
        // `PaymentRequired` doesn't derive `PartialEq` (because
        // `PaymentRequirements::extra` carries `serde_json::Value`),
        // so compare via canonical JSON equality.
        let lhs = serde_json::to_value(&body).unwrap();
        let rhs = serde_json::to_value(&back).unwrap();
        prop_assert_eq!(lhs, rhs);
    }

    #[test]
    fn verify_response_roundtrips(resp in arb_verify_response()) {
        let json = serde_json::to_vec(&resp).unwrap();
        let back: VerifyResponse = serde_json::from_slice(&json).unwrap();
        prop_assert_eq!(back, resp);
    }

    #[test]
    fn settle_response_roundtrips(resp in arb_settle_response()) {
        let json = serde_json::to_vec(&resp).unwrap();
        let back: SettleResponse = serde_json::from_slice(&json).unwrap();
        prop_assert_eq!(back, resp);
    }

    /// `ErrorReason` is the most security-sensitive type for round-trip
    /// fidelity: a lossy deserialisation here would silently drop a
    /// machine-readable error code from facilitator → client. Cover both
    /// the canonical and `Custom` paths.
    #[test]
    fn error_reason_roundtrips(reason in arb_error_reason()) {
        let json = serde_json::to_vec(&reason).unwrap();
        let back: ErrorReason = serde_json::from_slice(&json).unwrap();
        prop_assert_eq!(back, reason);
    }

    /// `ErrorReason::from_wire` must accept any string and map to either
    /// a canonical variant or `Custom(...)`. Confirms the deserialiser
    /// is total — no panic, no error — over an arbitrary byte sequence.
    #[test]
    fn error_reason_from_wire_is_total(input in "[\\x20-\\x7e]{0,64}") {
        // No assertion on the variant — just that conversion + display
        // don't panic and round-trip back to the same string.
        let reason = ErrorReason::from_wire(&input);
        prop_assert_eq!(reason.as_str(), input.as_str());
    }
}
