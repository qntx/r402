#![allow(unused_crate_dependencies, reason = "sibling tests consume them")]
#![allow(
    clippy::tests_outside_test_module,
    reason = "integration test binaries put #[test] fns at file scope"
)]
#![allow(
    clippy::indexing_slicing,
    clippy::items_after_statements,
    clippy::missing_assert_message,
    clippy::missing_const_for_fn,
    clippy::panic,
    clippy::unwrap_used,
    reason = "idiomatic test-code patterns"
)]

//! Closed payment-flow names, scheme-table resolve, and `extra` wire helpers.

use std::collections::HashMap;

use r402_protocol::payment::PaymentRequirements;
use r402_server::{
    PAYMENT_FLOWS, PaymentFlowConfig, PaymentFlowError, PaymentFlowName, PaymentFlowPhases,
    PaymentFlowScheme, ResolvedPaymentFlow, SDK_DEFAULT_ASSET_TRANSFER_METHOD, SettlePhase,
    apply_payment_flow_wire_extra, is_authorization_payment_flow, is_recognized_payment_flow,
    resolve_payment_flow, resolve_payment_flow_phases,
};
use serde_json::Value;

fn exact_evm_flows() -> HashMap<String, PaymentFlowConfig> {
    let auth_upfront = PaymentFlowConfig::new(
        vec![PaymentFlowName::Authorization, PaymentFlowName::Upfront],
        PaymentFlowName::Authorization,
    );
    let mut flows = HashMap::new();
    flows.insert("eip3009".into(), auth_upfront.clone());
    flows.insert("permit2".into(), auth_upfront);
    flows
}

fn requirements(extra: Value) -> PaymentRequirements {
    PaymentRequirements::new(
        "exact".into(),
        "eip155:8453".parse().unwrap(),
        "1000000".into(),
        "0xabc".into(),
        "0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913".into(),
        60,
    )
    .with_extra(extra)
}

fn resolve(
    flows: &HashMap<String, PaymentFlowConfig>,
    extra: Value,
) -> Result<ResolvedPaymentFlow, PaymentFlowError> {
    resolve_payment_flow(
        &PaymentFlowScheme {
            scheme: "exact",
            default_asset_transfer_method: "eip3009",
            payment_flows: flows,
        },
        &requirements(extra),
    )
}

#[test]
fn payment_flows_table_matches_closed_set() {
    assert_eq!(
        PAYMENT_FLOWS[0],
        (
            PaymentFlowName::Authorization,
            PaymentFlowPhases {
                verify_before_handler: true,
                settle_before_handler: false,
                settle_after_handler: true,
            }
        )
    );
    assert_eq!(
        PaymentFlowName::Upfront.phases(),
        PaymentFlowPhases {
            verify_before_handler: false,
            settle_before_handler: true,
            settle_after_handler: false,
        }
    );
    assert_eq!(
        resolve_payment_flow_phases(PaymentFlowName::Escrow),
        PaymentFlowPhases {
            verify_before_handler: false,
            settle_before_handler: true,
            settle_after_handler: true,
        }
    );
}

#[test]
fn settle_phase_wire_strings() {
    assert_eq!(SettlePhase::BeforeHandler.as_str(), "before-handler");
    assert_eq!(SettlePhase::AfterHandler.as_str(), "after-handler");
    assert_eq!(SettlePhase::Cancel.as_str(), "cancel");
    assert_eq!(
        serde_json::to_value(SettlePhase::AfterHandler).unwrap(),
        Value::String("after-handler".into())
    );
    assert_eq!(
        serde_json::from_value::<SettlePhase>(Value::String("before-handler".into())).unwrap(),
        SettlePhase::BeforeHandler
    );
    assert_eq!(
        "cancel".parse::<SettlePhase>().unwrap(),
        SettlePhase::Cancel
    );
    assert!(matches!(
        "after_handler".parse::<SettlePhase>(),
        Err(PaymentFlowError::UnknownSettlePhase { .. })
    ));
}

#[test]
fn payment_flow_name_wire_strings() {
    assert_eq!(
        serde_json::to_value(PaymentFlowName::Authorization).unwrap(),
        Value::String("authorization".into())
    );
    assert_eq!(
        "upfront".parse::<PaymentFlowName>().unwrap(),
        PaymentFlowName::Upfront
    );
    assert!(matches!(
        "Authorize".parse::<PaymentFlowName>(),
        Err(PaymentFlowError::UnknownPaymentFlow { ref flow }) if flow == "Authorize"
    ));
}

#[test]
fn resolve_omits_atm_and_payment_flow_to_scheme_defaults() {
    let got = resolve(&exact_evm_flows(), serde_json::json!({})).unwrap();
    assert_eq!(
        got,
        ResolvedPaymentFlow {
            asset_transfer_method: "eip3009".into(),
            payment_flow: PaymentFlowName::Authorization,
        }
    );
}

#[test]
fn resolve_explicit_defaults_match_omitted() {
    let got = resolve(
        &exact_evm_flows(),
        serde_json::json!({
            "assetTransferMethod": "eip3009",
            "paymentFlow": "authorization",
        }),
    )
    .unwrap();
    assert_eq!(got.asset_transfer_method, "eip3009");
    assert_eq!(got.payment_flow, PaymentFlowName::Authorization);
}

#[test]
fn resolve_upfront_and_permit2_atm() {
    let got = resolve(
        &exact_evm_flows(),
        serde_json::json!({
            "assetTransferMethod": "permit2",
            "paymentFlow": "upfront",
        }),
    )
    .unwrap();
    assert_eq!(
        got,
        ResolvedPaymentFlow {
            asset_transfer_method: "permit2".into(),
            payment_flow: PaymentFlowName::Upfront,
        }
    );
}

#[test]
fn resolve_throws_on_unknown_atm() {
    let err = resolve(
        &exact_evm_flows(),
        serde_json::json!({ "assetTransferMethod": "unknown" }),
    )
    .unwrap_err();
    assert!(matches!(
        err,
        PaymentFlowError::UnsupportedAssetTransferMethod {
            ref asset_transfer_method,
            ..
        } if asset_transfer_method == "unknown"
    ));
}

#[test]
fn resolve_throws_on_unsupported_flow() {
    let err = resolve(
        &exact_evm_flows(),
        serde_json::json!({ "paymentFlow": "escrow" }),
    )
    .unwrap_err();
    assert!(matches!(
        err,
        PaymentFlowError::UnsupportedPaymentFlow { ref requested, .. } if requested == "escrow"
    ));
}

#[test]
fn resolve_null_payment_flow_uses_default() {
    let got = resolve(
        &exact_evm_flows(),
        serde_json::json!({ "paymentFlow": null }),
    )
    .unwrap();
    assert_eq!(got.payment_flow, PaymentFlowName::Authorization);
}

#[test]
fn resolve_rejects_default_not_in_supported() {
    let mut flows = HashMap::new();
    flows.insert(
        "eip3009".into(),
        PaymentFlowConfig::new(
            vec![PaymentFlowName::Upfront],
            PaymentFlowName::Authorization,
        ),
    );
    let err = resolve(&flows, serde_json::json!({})).unwrap_err();
    assert!(matches!(
        err,
        PaymentFlowError::DefaultNotInSupported {
            ref asset_transfer_method,
            ..
        } if asset_transfer_method == "eip3009"
    ));
}

#[test]
fn apply_leaves_authorization_alone() {
    let extra = serde_json::json!({ "name": "USDC" });
    let next = apply_payment_flow_wire_extra(
        Some(&extra),
        &ResolvedPaymentFlow {
            asset_transfer_method: "eip3009".into(),
            payment_flow: PaymentFlowName::Authorization,
        },
    );
    assert_eq!(Value::Object(next), extra);
}

#[test]
fn apply_forces_non_authorization_payment_flow() {
    let upfront = apply_payment_flow_wire_extra(
        None,
        &ResolvedPaymentFlow {
            asset_transfer_method: "eip3009".into(),
            payment_flow: PaymentFlowName::Upfront,
        },
    );
    assert_eq!(
        upfront.get("paymentFlow").and_then(Value::as_str),
        Some("upfront")
    );
    let escrow = apply_payment_flow_wire_extra(
        None,
        &ResolvedPaymentFlow {
            asset_transfer_method: SDK_DEFAULT_ASSET_TRANSFER_METHOD.into(),
            payment_flow: PaymentFlowName::Escrow,
        },
    );
    assert_eq!(
        escrow.get("paymentFlow").and_then(Value::as_str),
        Some("escrow")
    );
    assert!(!escrow.contains_key("assetTransferMethod"));
}

#[test]
fn apply_strips_sdk_atm_sentinel() {
    let extra = serde_json::json!({
        "assetTransferMethod": SDK_DEFAULT_ASSET_TRANSFER_METHOD,
        "name": "USDC",
    });
    let next = apply_payment_flow_wire_extra(
        Some(&extra),
        &ResolvedPaymentFlow {
            asset_transfer_method: SDK_DEFAULT_ASSET_TRANSFER_METHOD.into(),
            payment_flow: PaymentFlowName::Authorization,
        },
    );
    assert_eq!(next.get("name").and_then(Value::as_str), Some("USDC"));
    assert!(!next.contains_key("assetTransferMethod"));
}

#[test]
fn recognized_payment_flow_accepts_omit_null_and_closed_names() {
    assert!(is_recognized_payment_flow(None));
    assert!(is_recognized_payment_flow(Some(&serde_json::json!({}))));
    assert!(is_recognized_payment_flow(Some(
        &serde_json::json!({"paymentFlow": null})
    )));
    for name in ["authorization", "upfront", "escrow"] {
        assert!(is_recognized_payment_flow(Some(
            &serde_json::json!({"paymentFlow": name})
        )));
    }
    assert!(!is_recognized_payment_flow(Some(
        &serde_json::json!({"paymentFlow": "future-flow"})
    )));
    assert!(is_authorization_payment_flow(None));
    assert!(!is_authorization_payment_flow(Some(
        &serde_json::json!({"paymentFlow": "upfront"})
    )));
}
