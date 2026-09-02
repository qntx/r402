//! Resource server, `paymentFlow`, and `SettlementMode` scheduler for the x402 payment protocol.

#![cfg_attr(docsrs, feature(doc_cfg))]
#![cfg_attr(
    test,
    allow(
        clippy::indexing_slicing,
        clippy::panic,
        reason = "unit tests panic on assertion failure"
    )
)]

mod cancel;
mod hooks;
mod matching;
mod payment_flow;
mod required;
mod resource;
mod scheme;
mod settle;
mod settlement;
mod verify;

pub use cancel::{CancellationGuard, build_failure_path_settlement_response};
pub use hooks::{
    AfterVerifyDecision, BeforeOpDecision, CancelReason, DynResourceServerHooks, HookPolicyError,
    PaymentHookContext, RESERVED_PAYMENT_FLOW_EXTRA_KEYS, ResourceServerHooks, SettleContext,
    SettleResponseCoreSnapshot, SettleResultContext, SkipHandlerDirective,
    VerifiedPaymentCanceledContext, VerifyResultContext, WirePaymentPayload,
    assert_accepts_additive_extra_after_scheme_enrich,
    assert_accepts_allowlisted_after_extension_enrich, assert_additive_payload_enrichment,
    assert_additive_settlement_extra, assert_settle_response_core_unchanged,
    is_vacant_string_field, merge_additive_settlement_extra, snapshot_payment_requirements_list,
    snapshot_settle_response_core,
};
pub use payment_flow::{
    PAYMENT_FLOWS, PaymentFlowConfig, PaymentFlowError, PaymentFlowName, PaymentFlowPhases,
    PaymentFlowScheme, ResolvedPaymentFlow, SDK_DEFAULT_ASSET_TRANSFER_METHOD, SettlePhase,
    apply_payment_flow_wire_extra, extra_payment_flow, is_authorization_payment_flow,
    is_recognized_payment_flow, resolve_payment_flow, resolve_payment_flow_phases,
};
pub use required::PaymentRequiredBuildContext;
pub use resource::ResourceServer;
pub use scheme::{DynSchemeNetworkServer, SchemeNetworkServer, SchemePaymentRequiredContext};
pub use settle::CompletedSettlement;
pub use settlement::{
    AfterHandler, BackgroundSettlementTracker, IncompatibleSettlementMode, ScheduledSettlement,
    SettlementMode, SettlementSchedule, finish, run, schedule,
};
pub use verify::VerifyPaymentOutcome;
