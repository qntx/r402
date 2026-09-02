//! Closed payment-flow names, settle phases, and `extra.paymentFlow` helpers.

use std::collections::HashMap;
use std::fmt::{self, Display, Formatter};
use std::str::FromStr;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::wire::PaymentRequirements;

/// SDK-only ATM key for schemes with no on-wire `assetTransferMethod`.
///
/// Never emit `assetTransferMethod: "default"` on the 402 wire.
pub const SDK_DEFAULT_ASSET_TRANSFER_METHOD: &str = "default";

/// Closed set of payment-flow orchestration modes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PaymentFlowName {
    /// Verify before the handler; settle after (`authorization`).
    Authorization,
    /// Settle before the handler; skip facilitator verify (`upfront`).
    Upfront,
    /// Settle before and after the handler (`escrow`).
    Escrow,
}

impl PaymentFlowName {
    /// All closed payment-flow names, in declaration order.
    pub const ALL: [Self; 3] = [Self::Authorization, Self::Upfront, Self::Escrow];

    /// Official wire string.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Authorization => "authorization",
            Self::Upfront => "upfront",
            Self::Escrow => "escrow",
        }
    }

    /// Official `PAYMENT_FLOWS` row for this name.
    #[must_use]
    pub const fn phases(self) -> PaymentFlowPhases {
        match self {
            Self::Authorization => PaymentFlowPhases {
                verify_before_handler: true,
                settle_before_handler: false,
                settle_after_handler: true,
            },
            Self::Upfront => PaymentFlowPhases {
                verify_before_handler: false,
                settle_before_handler: true,
                settle_after_handler: false,
            },
            Self::Escrow => PaymentFlowPhases {
                verify_before_handler: false,
                settle_before_handler: true,
                settle_after_handler: true,
            },
        }
    }
}

impl Display for PaymentFlowName {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for PaymentFlowName {
    type Err = PaymentFlowError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "authorization" => Ok(Self::Authorization),
            "upfront" => Ok(Self::Upfront),
            "escrow" => Ok(Self::Escrow),
            other => Err(PaymentFlowError::UnknownPaymentFlow {
                flow: other.to_owned(),
            }),
        }
    }
}

/// `extra.paymentFlow` when `extra` is a JSON object.
#[must_use]
pub fn extra_payment_flow(extra: Option<&Value>) -> Option<&Value> {
    extra.and_then(|value| value.get("paymentFlow"))
}

/// Whether `extra.paymentFlow` is absent/null or a closed payment-flow name.
///
/// Used by client selection (official `IsRecognizedPaymentFlow`).
#[must_use]
pub fn is_recognized_payment_flow(extra: Option<&Value>) -> bool {
    match extra_payment_flow(extra) {
        None | Some(Value::Null) => true,
        Some(Value::String(name)) => name.parse::<PaymentFlowName>().is_ok(),
        Some(_) => false,
    }
}

/// Whether the accept is authorization (omit, JSON null, or `"authorization"`).
#[must_use]
pub fn is_authorization_payment_flow(extra: Option<&Value>) -> bool {
    match extra_payment_flow(extra) {
        None | Some(Value::Null) => true,
        Some(Value::String(name)) => name == PaymentFlowName::Authorization.as_str(),
        Some(_) => false,
    }
}

/// Which settle invocation is running.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SettlePhase {
    /// Settle before the resource handler (`before-handler`).
    BeforeHandler,
    /// Settle after the resource handler (`after-handler`).
    AfterHandler,
    /// Refund/close settle from verified-payment cancellation (`cancel`).
    Cancel,
}

impl SettlePhase {
    /// Official wire string.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::BeforeHandler => "before-handler",
            Self::AfterHandler => "after-handler",
            Self::Cancel => "cancel",
        }
    }
}

impl Display for SettlePhase {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for SettlePhase {
    type Err = PaymentFlowError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "before-handler" => Ok(Self::BeforeHandler),
            "after-handler" => Ok(Self::AfterHandler),
            "cancel" => Ok(Self::Cancel),
            other => Err(PaymentFlowError::UnknownSettlePhase {
                phase: other.to_owned(),
            }),
        }
    }
}

/// Verify/settle phase flags for a [`PaymentFlowName`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[allow(
    clippy::struct_excessive_bools,
    reason = "official PAYMENT_FLOWS phase flags"
)]
pub struct PaymentFlowPhases {
    /// Run facilitator verify before the resource handler.
    pub verify_before_handler: bool,
    /// Run settle before the resource handler.
    pub settle_before_handler: bool,
    /// Run settle after the resource handler.
    pub settle_after_handler: bool,
}

/// Supported payment flows for one `assetTransferMethod`.
///
/// `default` must be a member of `supported` (checked by [`resolve_payment_flow`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaymentFlowConfig {
    /// Flows this ATM accepts.
    pub supported: Vec<PaymentFlowName>,
    /// Used when `extra.paymentFlow` is omitted.
    pub default: PaymentFlowName,
}

impl PaymentFlowConfig {
    /// Constructs a per-ATM flow table.
    #[must_use]
    pub const fn new(supported: Vec<PaymentFlowName>, default: PaymentFlowName) -> Self {
        Self { supported, default }
    }

    /// Official exact-scheme row: authorization + upfront, default authorization.
    #[must_use]
    pub fn authorization_and_upfront() -> Self {
        Self::new(
            vec![PaymentFlowName::Authorization, PaymentFlowName::Upfront],
            PaymentFlowName::Authorization,
        )
    }

    /// Official upto / batch-settlement row: authorization only.
    #[must_use]
    pub fn authorization_only() -> Self {
        Self::new(
            vec![PaymentFlowName::Authorization],
            PaymentFlowName::Authorization,
        )
    }
}

/// Scheme fields required to resolve ATM + payment flow.
#[derive(Debug, Clone, Copy)]
pub struct PaymentFlowScheme<'a> {
    /// Scheme name (e.g. `"exact"`).
    pub scheme: &'a str,
    /// ATM used when `extra.assetTransferMethod` is absent.
    pub default_asset_transfer_method: &'a str,
    /// Payment flows supported per ATM.
    pub payment_flows: &'a HashMap<String, PaymentFlowConfig>,
}

/// Result of [`resolve_payment_flow`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedPaymentFlow {
    /// Resolved `assetTransferMethod`.
    pub asset_transfer_method: String,
    /// Resolved payment flow.
    pub payment_flow: PaymentFlowName,
}

/// Closed `PAYMENT_FLOWS` table (authorization, upfront, escrow).
pub const PAYMENT_FLOWS: [(PaymentFlowName, PaymentFlowPhases); 3] = [
    (
        PaymentFlowName::Authorization,
        PaymentFlowName::Authorization.phases(),
    ),
    (PaymentFlowName::Upfront, PaymentFlowName::Upfront.phases()),
    (PaymentFlowName::Escrow, PaymentFlowName::Escrow.phases()),
];

/// Failures from payment-flow resolution or wire-name parsing.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PaymentFlowError {
    /// Scheme table has no row for the resolved ATM.
    #[error(
        "[x402] Scheme \"{scheme}\" does not support assetTransferMethod \"{asset_transfer_method}\". Supported: {supported}."
    )]
    UnsupportedAssetTransferMethod {
        /// Scheme name.
        scheme: String,
        /// Requested or default ATM.
        asset_transfer_method: String,
        /// Comma-separated ATM keys from the scheme table.
        supported: String,
    },
    /// `paymentFlows[atm].default` is not listed in `supported`.
    #[error(
        "[x402] Scheme \"{scheme}\" paymentFlows[\"{asset_transfer_method}\"].default is not in supported."
    )]
    DefaultNotInSupported {
        /// Scheme name.
        scheme: String,
        /// ATM whose default is invalid.
        asset_transfer_method: String,
    },
    /// Requested `extra.paymentFlow` is not in the ATM's `supported` list.
    #[error(
        "[x402] Scheme \"{scheme}\" assetTransferMethod \"{asset_transfer_method}\" does not support paymentFlow \"{requested}\". Supported: {supported} (default: {default})."
    )]
    UnsupportedPaymentFlow {
        /// Scheme name.
        scheme: String,
        /// Resolved ATM.
        asset_transfer_method: String,
        /// Requested flow as it appeared on extra (or the invalid default).
        requested: String,
        /// Comma-separated supported flow names.
        supported: String,
        /// ATM default flow.
        default: String,
    },
    /// String is not a closed [`PaymentFlowName`].
    #[error(
        "[x402] Unknown payment flow \"{flow}\". Expected one of: authorization, upfront, escrow."
    )]
    UnknownPaymentFlow {
        /// Rejected token.
        flow: String,
    },
    /// String is not a closed [`SettlePhase`].
    #[error(
        "[x402] Unknown settle phase \"{phase}\". Expected one of: before-handler, after-handler, cancel."
    )]
    UnknownSettlePhase {
        /// Rejected token.
        phase: String,
    },
    /// No [`super::SchemeNetworkServer`] is registered for this scheme/network.
    #[error("[x402] No server implementation registered for scheme: {scheme}, network: {network}")]
    UnregisteredScheme {
        /// Requested scheme name.
        scheme: String,
        /// Requested CAIP-2 network.
        network: String,
    },
}

/// Resolves the phase table for a payment flow name.
#[must_use]
pub const fn resolve_payment_flow_phases(flow: PaymentFlowName) -> PaymentFlowPhases {
    flow.phases()
}

/// Resolves `assetTransferMethod` and `paymentFlow` from a scheme table and requirements.
///
/// Omit ATM → `scheme.default_asset_transfer_method`. Omit `paymentFlow` → that ATM's default.
///
/// # Errors
///
/// Returns [`PaymentFlowError`] when the ATM is missing from the table, the table
/// default is not in `supported`, or the requested flow is not supported.
pub fn resolve_payment_flow(
    scheme: &PaymentFlowScheme<'_>,
    requirements: &PaymentRequirements,
) -> Result<ResolvedPaymentFlow, PaymentFlowError> {
    let atm = extra_string(requirements, "assetTransferMethod")
        .unwrap_or(scheme.default_asset_transfer_method);

    let Some(config) = scheme.payment_flows.get(atm) else {
        return Err(PaymentFlowError::UnsupportedAssetTransferMethod {
            scheme: scheme.scheme.to_owned(),
            asset_transfer_method: atm.to_owned(),
            supported: join_sorted_keys(scheme.payment_flows),
        });
    };

    if !config.supported.contains(&config.default) {
        return Err(PaymentFlowError::DefaultNotInSupported {
            scheme: scheme.scheme.to_owned(),
            asset_transfer_method: atm.to_owned(),
        });
    }

    let flow = match requested_flow(requirements) {
        None => config.default,
        Some(label) => match PaymentFlowName::from_str(&label) {
            Ok(name) if config.supported.contains(&name) => name,
            Ok(name) => {
                return Err(unsupported_flow(scheme, atm, name.as_str(), config));
            }
            Err(_) => return Err(unsupported_flow(scheme, atm, &label, config)),
        },
    };

    Ok(ResolvedPaymentFlow {
        asset_transfer_method: atm.to_owned(),
        payment_flow: flow,
    })
}

/// Applies resolved payment-flow rules to 402 `extra`:
///
/// - Strip the SDK ATM sentinel `"default"` (never on the wire).
/// - When the resolved flow is not `authorization`, set `extra.paymentFlow`.
#[must_use]
pub fn apply_payment_flow_wire_extra(
    extra: Option<&Value>,
    resolved: &ResolvedPaymentFlow,
) -> Map<String, Value> {
    let mut next = extra
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    let extra_atm = next.get("assetTransferMethod").and_then(Value::as_str);
    if resolved.asset_transfer_method == SDK_DEFAULT_ASSET_TRANSFER_METHOD
        || extra_atm == Some(SDK_DEFAULT_ASSET_TRANSFER_METHOD)
    {
        next.remove("assetTransferMethod");
    }
    if resolved.payment_flow != PaymentFlowName::Authorization {
        next.insert(
            "paymentFlow".to_owned(),
            Value::String(resolved.payment_flow.as_str().to_owned()),
        );
    }
    next
}

fn extra_string<'a>(requirements: &'a PaymentRequirements, key: &str) -> Option<&'a str> {
    requirements
        .extra
        .as_ref()
        .and_then(|extra| extra.get(key))
        .and_then(Value::as_str)
}

fn requested_flow(requirements: &PaymentRequirements) -> Option<String> {
    let value = requirements
        .extra
        .as_ref()
        .and_then(|extra| extra.get("paymentFlow"))?;
    match value {
        Value::Null => None,
        Value::String(s) => Some(s.clone()),
        other => Some(other.to_string()),
    }
}

fn unsupported_flow(
    scheme: &PaymentFlowScheme<'_>,
    atm: &str,
    requested: &str,
    config: &PaymentFlowConfig,
) -> PaymentFlowError {
    PaymentFlowError::UnsupportedPaymentFlow {
        scheme: scheme.scheme.to_owned(),
        asset_transfer_method: atm.to_owned(),
        requested: requested.to_owned(),
        supported: join_names(&config.supported),
        default: config.default.as_str().to_owned(),
    }
}

fn join_names(names: &[PaymentFlowName]) -> String {
    names
        .iter()
        .map(|name| name.as_str())
        .collect::<Vec<_>>()
        .join(", ")
}

fn join_sorted_keys(flows: &HashMap<String, PaymentFlowConfig>) -> String {
    let mut keys: Vec<&str> = flows.keys().map(String::as_str).collect();
    keys.sort_unstable();
    keys.join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn payment_flows_table_matches_official() {
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
    fn resolve_ignores_non_string_atm() {
        let got = resolve(
            &exact_evm_flows(),
            serde_json::json!({ "assetTransferMethod": 1 }),
        )
        .unwrap();
        assert_eq!(got.asset_transfer_method, "eip3009");
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
        let msg = err.to_string();
        assert!(
            msg.contains("does not support assetTransferMethod \"unknown\""),
            "{msg}"
        );
        assert!(msg.contains("eip3009"), "{msg}");
        assert!(msg.contains("permit2"), "{msg}");
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
        let msg = err.to_string();
        assert!(
            msg.contains("does not support paymentFlow \"escrow\""),
            "{msg}"
        );
        assert!(msg.contains("default: authorization"), "{msg}");
    }

    #[test]
    fn resolve_throws_on_unknown_flow_name() {
        let err = resolve(
            &exact_evm_flows(),
            serde_json::json!({ "paymentFlow": "not-a-flow" }),
        )
        .unwrap_err();
        assert!(matches!(
            err,
            PaymentFlowError::UnsupportedPaymentFlow { ref requested, .. }
                if requested == "not-a-flow"
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
        let msg = err.to_string();
        assert!(
            msg.contains("paymentFlows[\"eip3009\"].default is not in supported"),
            "{msg}"
        );
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
        assert!(!is_recognized_payment_flow(Some(
            &serde_json::json!({"paymentFlow": 1})
        )));
        assert!(is_authorization_payment_flow(None));
        assert!(is_authorization_payment_flow(Some(
            &serde_json::json!({"paymentFlow": "authorization"})
        )));
        assert!(!is_authorization_payment_flow(Some(
            &serde_json::json!({"paymentFlow": "upfront"})
        )));
    }
}
