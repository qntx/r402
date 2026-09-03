//! Closed payment-flow names, settle phases, and scheme-table resolution.

mod extra;

use std::collections::HashMap;
use std::fmt::{self, Display, Formatter};
use std::str::FromStr;

pub use extra::{
    apply_payment_flow_wire_extra, extra_payment_flow, is_authorization_payment_flow,
    is_recognized_payment_flow,
};
use extra::{extra_string, requested_flow};
use r402_protocol::payment::PaymentRequirements;
use serde::{Deserialize, Serialize};

/// SDK-only ATM key for schemes with no on-wire `assetTransferMethod`.
///
/// Never emit `assetTransferMethod: "default"` on the 402 wire.
pub const SDK_DEFAULT_ASSET_TRANSFER_METHOD: &str = "default";

/// Closed set of payment-flow names (when on-chain value moves relative to the handler).
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

    /// Wire string.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Authorization => "authorization",
            Self::Upfront => "upfront",
            Self::Escrow => "escrow",
        }
    }

    /// Phase flags for this name.
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
    /// Wire string.
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
    reason = "PAYMENT_FLOWS phase flags are a closed triple"
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

    /// Exact-scheme row: authorization + upfront, default authorization.
    #[must_use]
    pub fn authorization_and_upfront() -> Self {
        Self::new(
            vec![PaymentFlowName::Authorization, PaymentFlowName::Upfront],
            PaymentFlowName::Authorization,
        )
    }

    /// Upto / batch-settlement row: authorization only.
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
    /// No [`crate::SchemeNetworkServer`] is registered for this scheme/network.
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
