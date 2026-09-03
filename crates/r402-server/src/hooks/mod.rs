//! Resource-server lifecycle hooks and enrichment policy.

mod policy;
mod resource;
mod snapshot;

pub use policy::{
    HookPolicyError, RESERVED_PAYMENT_FLOW_EXTRA_KEYS,
    assert_accepts_additive_extra_after_scheme_enrich,
    assert_accepts_allowlisted_after_extension_enrich, assert_additive_payload_enrichment,
    assert_additive_settlement_extra, is_vacant_string_field, merge_additive_settlement_extra,
};
pub use resource::{
    AfterVerifyDecision, BeforeOpDecision, CancelReason, DynResourceServerHooks,
    PaymentHookContext, ResourceServerHooks, SettleContext, SettleResultContext,
    SkipHandlerDirective, VerifiedPaymentCanceledContext, VerifyResultContext, WirePaymentPayload,
};
pub use snapshot::{
    SettleResponseCoreSnapshot, assert_settle_response_core_unchanged,
    snapshot_payment_requirements_list, snapshot_settle_response_core,
};
