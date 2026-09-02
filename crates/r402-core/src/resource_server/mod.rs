//! Resource-server orchestration shared by HTTP and MCP transports.
//!
//! Mirrors the official Go/TS `X402ResourceServer` **V2** surface used by
//! transports: match requirements, verify, settle, and lifecycle hooks
//! including verified-payment cancellation.
//!
//! This type does **not** own pricing or route maps; scheme handlers live in
//! [`crate::scheme::SchemeRegistry`] (or a remote facilitator). The resource
//! server only sequences wire construction and hooks against a [`Facilitator`].

mod hook_policy;
mod hooks;
mod payment_flow;

use std::str::FromStr;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use compact_str::CompactString;
pub use hook_policy::{
    HookPolicyError, RESERVED_PAYMENT_FLOW_EXTRA_KEYS, SettleResponseCoreSnapshot,
    assert_accepts_additive_extra_after_scheme_enrich,
    assert_accepts_allowlisted_after_extension_enrich, assert_additive_payload_enrichment,
    assert_additive_settlement_extra, assert_settle_response_core_unchanged,
    is_vacant_string_field, merge_additive_settlement_extra, snapshot_payment_requirements_list,
    snapshot_settle_response_core,
};
pub use hooks::{
    AfterVerifyDecision, BeforeOpDecision, CancelReason, DynResourceServerHooks,
    PaymentHookContext, ResourceServerHooks, SettleContext, SettleResultContext,
    SkipHandlerDirective, VerifiedPaymentCanceledContext, VerifyResultContext, WirePaymentPayload,
};
pub use payment_flow::{
    PAYMENT_FLOWS, PaymentFlowConfig, PaymentFlowError, PaymentFlowName, PaymentFlowPhases,
    PaymentFlowScheme, ResolvedPaymentFlow, SDK_DEFAULT_ASSET_TRANSFER_METHOD, SettlePhase,
    apply_payment_flow_wire_extra, resolve_payment_flow, resolve_payment_flow_phases,
};
use serde::Serialize;
use serde_json::Value;

use crate::error::{FacilitatorError, VerificationError};
use crate::extensions::{AdvertiseContext, Extension, ExtensionRegistry};
use crate::facilitator::FailureRecovery;
use crate::facilitator::{DynFacilitator, Facilitator};
use crate::wire::{
    Extensions, PaymentRequired, PaymentRequirements, ResourceInfo, SettleRequest, SettleResponse,
    SupportedResponse, TypedVerifyRequest, V2, VerifyRequest, VerifyResponse,
    find_matching_requirements,
};
use crate::wire::{
    SettlementOverrides, asset_decimals_from_extra, resolve_settlement_override_amount,
};

/// Successful verify path outcome (after hooks).
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct VerifyPaymentOutcome {
    /// Facilitator (or skipped/recovered) verify response.
    pub response: VerifyResponse,
    /// When set, the transport must skip the resource handler and settle inline.
    pub skip_handler: Option<SkipHandlerDirective>,
}

/// Inputs for [`ResourceServer::create_payment_required_response`].
#[derive(Debug, Clone)]
pub struct PaymentRequiredBuildContext {
    /// Resource metadata placed on the 402 body.
    pub resource: ResourceInfo,
    /// Optional error string (`PaymentRequired.error`).
    pub error: Option<CompactString>,
    /// Declared extension entries copied onto the 402 body before advertise.
    pub extensions: Extensions,
    /// Facilitator `/supported` snapshot (scheme enrich reads this later).
    pub supported: SupportedResponse,
    /// Failed payment payload, when building a 402 after a paid attempt.
    pub payment_payload: Option<WirePaymentPayload>,
}

/// Server-side payment orchestrator (official `X402ResourceServer`, V2-only).
pub struct ResourceServer {
    facilitator: Arc<dyn DynFacilitator>,
    hooks: Vec<Arc<dyn DynResourceServerHooks>>,
    extensions: ExtensionRegistry,
}

impl Clone for ResourceServer {
    fn clone(&self) -> Self {
        Self {
            facilitator: Arc::clone(&self.facilitator),
            hooks: self.hooks.clone(),
            extensions: self.extensions.clone(),
        }
    }
}

impl std::fmt::Debug for ResourceServer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ResourceServer")
            .field("hooks", &self.hooks.len())
            .field("extensions", &self.extensions.len())
            .finish_non_exhaustive()
    }
}

impl ResourceServer {
    /// Creates a resource server over any [`Facilitator`].
    #[must_use]
    pub fn new<F>(facilitator: Arc<F>) -> Self
    where
        F: Facilitator + 'static,
    {
        let erased: Arc<dyn DynFacilitator> = facilitator;
        Self {
            facilitator: erased,
            hooks: Vec::new(),
            extensions: ExtensionRegistry::new(),
        }
    }

    /// Creates from an already-erased facilitator handle.
    #[must_use]
    pub fn from_dyn(facilitator: Arc<dyn DynFacilitator>) -> Self {
        Self {
            facilitator,
            hooks: Vec::new(),
            extensions: ExtensionRegistry::new(),
        }
    }

    /// Registers a lifecycle hook. Returns `self` for builder chaining.
    #[must_use]
    pub fn with_hook(mut self, hook: impl ResourceServerHooks + 'static) -> Self {
        self.hooks.push(Arc::new(hook));
        self
    }

    /// Registers a lifecycle hook after construction.
    pub fn add_hook(&mut self, hook: impl ResourceServerHooks + 'static) {
        self.hooks.push(Arc::new(hook));
    }

    /// Registers a protocol extension advertised on 402 responses.
    #[must_use]
    pub fn with_extension(mut self, extension: impl Extension + 'static) -> Self {
        self.extensions.register(extension);
        self
    }

    /// Number of registered resource-server hooks.
    #[must_use]
    pub fn hook_count(&self) -> usize {
        self.hooks.len()
    }

    /// Returns a clone of the inner facilitator handle.
    #[must_use]
    pub fn facilitator(&self) -> Arc<dyn DynFacilitator> {
        Arc::clone(&self.facilitator)
    }

    /// Official `FindMatchingRequirements`.
    ///
    /// Method form matches the Go `X402ResourceServer` surface even though
    /// matching is pure over the arguments.
    #[must_use]
    #[allow(
        clippy::unused_self,
        reason = "API parity with Go X402ResourceServer.FindMatchingRequirements"
    )]
    pub fn find_matching_requirements<'a>(
        &self,
        available: &'a [PaymentRequirements],
        payload: &WirePaymentPayload,
    ) -> Option<&'a PaymentRequirements> {
        find_matching_requirements(available, &payload.accepted)
    }

    /// Official `CreatePaymentRequiredResponse`: one 402 body used for both
    /// unpaid challenges and paid matching.
    ///
    /// # Errors
    ///
    /// [`FacilitatorError::Internal`] when hook policy or payment-flow wire
    /// extra application fails (fail-closed).
    #[allow(
        clippy::unused_async,
        reason = "official createPaymentRequiredResponse is async"
    )]
    pub async fn create_payment_required_response(
        &self,
        accepts: Vec<PaymentRequirements>,
        ctx: PaymentRequiredBuildContext,
    ) -> Result<PaymentRequired, FacilitatorError> {
        let PaymentRequiredBuildContext {
            resource,
            error,
            extensions,
            supported: _,
            payment_payload: _,
        } = ctx;
        let mut response = PaymentRequired::new(resource)
            .with_accepts(accepts)
            .with_extensions(extensions);
        if let Some(error) = error {
            response = response.with_error(error);
        }
        advertise_registered_extensions(&self.extensions, &mut response)?;
        apply_payment_flow_extras(&mut response.accepts)?;
        Ok(response)
    }

    /// Official `VerifyPayment`: before hooks → facilitator → after hooks.
    ///
    /// # Errors
    ///
    /// Propagates facilitator errors (unless recovered), before-hook aborts,
    /// and after-verify aborts (`FacilitatorError::Aborted`).
    pub async fn verify_payment(
        &self,
        payload: &WirePaymentPayload,
        requirements: &PaymentRequirements,
    ) -> Result<VerifyPaymentOutcome, FacilitatorError> {
        let payment = PaymentHookContext {
            payload: payload.clone(),
            requirements: requirements.clone(),
        };

        // before_verify: first Abort / Skip wins
        let mut skipped: Option<VerifyResponse> = None;
        for hook in &self.hooks {
            match hook.before_verify(&payment).await {
                BeforeOpDecision::Continue => {}
                BeforeOpDecision::Abort { reason, message } => {
                    return Err(FacilitatorError::Aborted { reason, message });
                }
                BeforeOpDecision::Skip { result } => {
                    skipped = Some(result);
                    break;
                }
            }
        }

        let response = if let Some(local) = skipped {
            local
        } else {
            self.call_verify_with_failure_hooks(&payment, payload, requirements)
                .await?
        };

        // Official Go: isValid:false is a hard verify failure that runs
        // onVerifyFailure (security: reachable facilitator ≠ payment valid).
        // Unrecovered invalids skip after_verify.
        if !response.is_valid() {
            return Ok(VerifyPaymentOutcome {
                response,
                skip_handler: None,
            });
        }

        let result_ctx = VerifyResultContext {
            payment: payment.clone(),
            result: response.clone(),
        };

        let mut skip_handler: Option<SkipHandlerDirective> = None;
        for hook in &self.hooks {
            match hook.after_verify(&result_ctx).await {
                AfterVerifyDecision::Continue => {}
                AfterVerifyDecision::Abort { reason, message } => {
                    self.notify_verified_payment_canceled(
                        payload,
                        requirements,
                        CancelReason::AfterVerifyAborted,
                        Some(message.as_str()),
                        None,
                    )
                    .await;
                    return Err(FacilitatorError::Aborted { reason, message });
                }
                AfterVerifyDecision::SkipHandler {
                    response: directive,
                } => {
                    // Last SkipHandler wins (matches Go).
                    skip_handler = Some(directive);
                }
            }
        }

        Ok(VerifyPaymentOutcome {
            response,
            skip_handler,
        })
    }

    /// Official `SettlePayment`: optional amount overrides → before hooks →
    /// facilitator → after hooks.
    ///
    /// Facilitator settle is attempted twice only when the first `Ok` result is
    /// `settlement_pending` with a non-empty `transaction`; no sleep.
    ///
    /// `overrides` applies partial settlement (upto): atomic / percent / dollar
    /// amounts are resolved against `requirements.amount` before hooks see the
    /// payment context (matches Go `SettlePaymentWithExtensions`).
    ///
    /// `phase` is required ([`SettlePhase`]); hooks receive it on
    /// [`SettleContext`].
    ///
    /// # Errors
    ///
    /// Propagates facilitator / abort / invalid-override errors unless a
    /// failure hook recovers.
    pub async fn settle_payment(
        &self,
        payload: &WirePaymentPayload,
        requirements: &PaymentRequirements,
        overrides: Option<&SettlementOverrides>,
        phase: SettlePhase,
    ) -> Result<SettleResponse, FacilitatorError> {
        let effective = apply_settlement_overrides(requirements, overrides)?;
        let settle_ctx = SettleContext {
            payment: PaymentHookContext {
                payload: payload.clone(),
                requirements: effective,
            },
            declared_extensions: Extensions::new(),
            phase,
        };

        let mut skipped: Option<SettleResponse> = None;
        for hook in &self.hooks {
            match hook.before_settle(&settle_ctx).await {
                BeforeOpDecision::Continue => {}
                BeforeOpDecision::Abort { reason, message } => {
                    return Err(FacilitatorError::Aborted { reason, message });
                }
                BeforeOpDecision::Skip { result } => {
                    skipped = Some(result);
                    break;
                }
            }
        }

        let response = if let Some(local) = skipped {
            local
        } else {
            self.call_settle_with_failure_hooks(&settle_ctx).await?
        };

        // Official: afterSettle fires for settle results returned as Ok
        // (including unsuccessful settle responses from the facilitator).
        let result_ctx = SettleResultContext {
            settle: settle_ctx,
            result: response.clone(),
        };
        for hook in &self.hooks {
            hook.after_settle(&result_ctx).await;
        }

        Ok(response)
    }

    /// Fires `on_verified_payment_canceled` for every registered hook.
    ///
    /// Transports call this when a handler fails after a successful verify
    /// (or when an after-verify abort already did — safe to call again via
    /// [`CancellationGuard`]).
    pub async fn notify_verified_payment_canceled(
        &self,
        payload: &WirePaymentPayload,
        requirements: &PaymentRequirements,
        reason: CancelReason,
        error: Option<&str>,
        response_status: Option<u16>,
    ) {
        if self.hooks.is_empty() {
            return;
        }
        let ctx = VerifiedPaymentCanceledContext {
            payment: PaymentHookContext {
                payload: payload.clone(),
                requirements: requirements.clone(),
            },
            reason,
            error: error.map(str::to_owned),
            response_status,
        };
        for hook in &self.hooks {
            hook.on_verified_payment_canceled(&ctx).await;
        }
    }

    /// Builds a one-shot cancellation dispatcher (first call wins).
    #[must_use]
    pub fn cancellation_guard(
        &self,
        payload: WirePaymentPayload,
        requirements: PaymentRequirements,
    ) -> CancellationGuard {
        CancellationGuard {
            server: self.clone(),
            payload,
            requirements,
            fired: AtomicBool::new(false),
        }
    }

    async fn call_verify_with_failure_hooks(
        &self,
        payment: &PaymentHookContext,
        payload: &WirePaymentPayload,
        requirements: &PaymentRequirements,
    ) -> Result<VerifyResponse, FacilitatorError> {
        let request = build_verify_request(payload, requirements)?;
        #[cfg(feature = "metrics")]
        let started = std::time::Instant::now();
        let result = match DynFacilitator::verify(self.facilitator.as_ref(), request).await {
            Ok(r) if r.is_valid() => Ok(r),
            Ok(invalid) => {
                let error = facilitator_error_from_invalid(&invalid);
                self.recover_verify(payment, error).await.or(Ok(invalid))
            }
            Err(error) => self.recover_verify(payment, error).await,
        };
        #[cfg(feature = "metrics")]
        crate::metrics::record_verify(&result, started.elapsed());
        result
    }

    async fn recover_verify(
        &self,
        payment: &PaymentHookContext,
        error: FacilitatorError,
    ) -> Result<VerifyResponse, FacilitatorError> {
        for hook in &self.hooks {
            if let FailureRecovery::Recovered(r) = hook.on_verify_failure(payment, &error).await {
                return Ok(r);
            }
        }
        Err(error)
    }

    async fn call_settle_with_failure_hooks(
        &self,
        settle: &SettleContext,
    ) -> Result<SettleResponse, FacilitatorError> {
        let request = build_settle_request(&settle.payment.payload, &settle.payment.requirements)?;
        #[cfg(feature = "metrics")]
        let started = std::time::Instant::now();
        let result = match self.settle_with_pending_retry(request).await {
            Ok(r) => Ok(r),
            Err(error) => self.recover_settle(settle, error).await,
        };
        #[cfg(feature = "metrics")]
        crate::metrics::record_settle(&result, started.elapsed());
        result
    }

    /// Calls facilitator settle once, then retries identically iff the first
    /// result is `Ok(SettleResponse::Failure)` with
    /// [`crate::error::ErrorReason::SettlementPending`] and a non-empty
    /// transaction hash. `Err` is never retried.
    ///
    /// No sleep or backoff. Capped at one retry. Emit pending only as that
    /// `Ok(Failure)` after writing the hash into a
    /// [`crate::PendingSettlementStore`].
    async fn settle_with_pending_retry(
        &self,
        request: SettleRequest,
    ) -> Result<SettleResponse, FacilitatorError> {
        let first = DynFacilitator::settle(self.facilitator.as_ref(), request.clone()).await;
        match &first {
            Ok(response) if response.is_retryable_settlement_pending() => {
                DynFacilitator::settle(self.facilitator.as_ref(), request).await
            }
            _ => first,
        }
    }

    async fn recover_settle(
        &self,
        settle: &SettleContext,
        error: FacilitatorError,
    ) -> Result<SettleResponse, FacilitatorError> {
        for hook in &self.hooks {
            if let FailureRecovery::Recovered(r) = hook.on_settle_failure(settle, &error).await {
                return Ok(r);
            }
        }
        Err(error)
    }
}

/// Ensures `on_verified_payment_canceled` runs at most once for a payment.
#[derive(Debug)]
pub struct CancellationGuard {
    server: ResourceServer,
    payload: WirePaymentPayload,
    requirements: PaymentRequirements,
    fired: AtomicBool,
}

impl CancellationGuard {
    /// Fires cancel hooks if not already fired.
    pub async fn cancel(
        &self,
        reason: CancelReason,
        error: Option<&str>,
        response_status: Option<u16>,
    ) {
        if self
            .fired
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_err()
        {
            return;
        }
        self.server
            .notify_verified_payment_canceled(
                &self.payload,
                &self.requirements,
                reason,
                error,
                response_status,
            )
            .await;
    }

    /// Returns `true` when cancel has already been dispatched.
    #[must_use]
    pub fn has_fired(&self) -> bool {
        self.fired.load(Ordering::SeqCst)
    }
}

fn build_verify_request(
    payload: &WirePaymentPayload,
    requirements: &PaymentRequirements,
) -> Result<VerifyRequest, FacilitatorError> {
    let typed = TypedVerifyRequest {
        x402_version: V2,
        payment_payload: payload.clone(),
        payment_requirements: requirements.clone(),
    };
    to_verify_request(&typed)
}

fn build_settle_request(
    payload: &WirePaymentPayload,
    requirements: &PaymentRequirements,
) -> Result<SettleRequest, FacilitatorError> {
    let typed = TypedVerifyRequest {
        x402_version: V2,
        payment_payload: payload.clone(),
        payment_requirements: requirements.clone(),
    };
    let verify = to_verify_request(&typed)?;
    Ok(SettleRequest::from(verify.into_json()))
}

fn to_verify_request<T: Serialize>(typed: &T) -> Result<VerifyRequest, FacilitatorError> {
    let json = serde_json::to_value(typed).map_err(FacilitatorError::internal)?;
    Ok(VerifyRequest::from(json))
}

/// Applies official settlement overrides onto a clone of `requirements`.
fn apply_settlement_overrides(
    requirements: &PaymentRequirements,
    overrides: Option<&SettlementOverrides>,
) -> Result<PaymentRequirements, FacilitatorError> {
    let Some(overrides) = overrides else {
        return Ok(requirements.clone());
    };
    let Some(raw) = overrides
        .amount
        .as_ref()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
    else {
        return Ok(requirements.clone());
    };
    let decimals = asset_decimals_from_extra(requirements.extra.as_ref());
    let resolved = resolve_settlement_override_amount(raw, requirements.amount.as_str(), decimals)
        .map_err(|e| {
            FacilitatorError::Verification(VerificationError::InvalidFormat(format!(
                "invalid settlement override: {e}"
            )))
        })?;
    let mut effective = requirements.clone();
    effective.amount = resolved.into();
    Ok(effective)
}

fn advertise_registered_extensions(
    registry: &ExtensionRegistry,
    response: &mut PaymentRequired,
) -> Result<(), FacilitatorError> {
    for ext in registry.iter() {
        let baseline = snapshot_payment_requirements_list(&response.accepts);
        if response.extensions.get(ext.id()).is_none()
            && let Some(entry) = ext.advertise(&AdvertiseContext { requirement: None })
        {
            response.extensions.insert(ext.id(), entry);
        }
        assert_accepts_allowlisted_after_extension_enrich(&baseline, &response.accepts, ext.id())
            .map_err(FacilitatorError::internal)?;
    }
    Ok(())
}

fn apply_payment_flow_extras(accepts: &mut [PaymentRequirements]) -> Result<(), FacilitatorError> {
    for accept in accepts {
        let atm = accept
            .extra
            .as_ref()
            .and_then(|extra| extra.get("assetTransferMethod"))
            .and_then(Value::as_str)
            .unwrap_or(SDK_DEFAULT_ASSET_TRANSFER_METHOD);
        let flow = match accept
            .extra
            .as_ref()
            .and_then(|extra| extra.get("paymentFlow"))
        {
            None | Some(Value::Null) => PaymentFlowName::Authorization,
            Some(Value::String(label)) => {
                PaymentFlowName::from_str(label).map_err(FacilitatorError::internal)?
            }
            Some(other) => {
                PaymentFlowName::from_str(&other.to_string()).map_err(FacilitatorError::internal)?
            }
        };
        let resolved = ResolvedPaymentFlow {
            asset_transfer_method: atm.to_owned(),
            payment_flow: flow,
        };
        let next = apply_payment_flow_wire_extra(accept.extra.as_ref(), &resolved);
        accept.extra = if next.is_empty() {
            None
        } else {
            Some(Value::Object(next))
        };
    }
    Ok(())
}

fn facilitator_error_from_invalid(invalid: &VerifyResponse) -> FacilitatorError {
    match invalid {
        VerifyResponse::Invalid {
            reason, message, ..
        } => {
            let detail = message
                .as_ref()
                .map_or_else(|| reason.to_string(), |m| format!("{reason}: {m}"));
            FacilitatorError::Verification(VerificationError::InvalidFormat(detail))
        }
        VerifyResponse::Valid { .. } => FacilitatorError::Verification(
            VerificationError::InvalidFormat("expected invalid verify response".into()),
        ),
    }
}

#[cfg(test)]
#[allow(
    clippy::excessive_nesting,
    reason = "hook + mock facilitator tests nest async bodies"
)]
mod tests {
    use std::future::Future;
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use compact_str::CompactString;

    use super::*;
    use crate::extensions::{AdvertiseContext, Extension};
    use crate::wire::{ExtensionEntry, Extensions, SupportedResponse};

    struct MockFacilitator {
        verifies: AtomicUsize,
        settles: AtomicUsize,
        fail_verify: bool,
        fail_settle: bool,
    }

    impl Facilitator for MockFacilitator {
        fn verify(
            &self,
            _request: VerifyRequest,
        ) -> impl Future<Output = Result<VerifyResponse, FacilitatorError>> + Send {
            self.verifies.fetch_add(1, Ordering::SeqCst);
            let result = if self.fail_verify {
                Err(FacilitatorError::Onchain("mock verify".into()))
            } else {
                Ok(VerifyResponse::valid("0xpayer"))
            };
            std::future::ready(result)
        }

        fn settle(
            &self,
            _request: SettleRequest,
        ) -> impl Future<Output = Result<SettleResponse, FacilitatorError>> + Send {
            self.settles.fetch_add(1, Ordering::SeqCst);
            let result = if self.fail_settle {
                Err(FacilitatorError::Onchain("mock settle".into()))
            } else {
                Ok(SettleResponse::Success {
                    payer: "0xpayer".into(),
                    transaction: "0xtx".into(),
                    network: "eip155:1".into(),
                    amount: Some("1".into()),
                    extensions: Extensions::new(),
                    extra: None,
                })
            };
            std::future::ready(result)
        }

        fn supported(
            &self,
        ) -> impl Future<Output = Result<SupportedResponse, FacilitatorError>> + Send {
            std::future::ready(Ok(SupportedResponse::default()))
        }
    }

    fn sample_payload() -> WirePaymentPayload {
        let req = PaymentRequirements::new(
            "exact".into(),
            "eip155:1".parse().unwrap(),
            "1".into(),
            "0xa".into(),
            "0xb".into(),
            60,
        );
        WirePaymentPayload::new(req, serde_json::json!({"k": 1}))
    }

    fn mock_ok() -> Arc<MockFacilitator> {
        Arc::new(MockFacilitator {
            verifies: AtomicUsize::new(0),
            settles: AtomicUsize::new(0),
            fail_verify: false,
            fail_settle: false,
        })
    }

    #[tokio::test]
    async fn verify_and_settle_invoke_facilitator() {
        let mock = mock_ok();
        let rs = ResourceServer::new(Arc::clone(&mock));
        let payload = sample_payload();
        let req = payload.accepted.clone();
        let out = rs.verify_payment(&payload, &req).await.unwrap();
        assert!(out.response.is_valid());
        assert!(out.skip_handler.is_none());
        assert!(
            rs.settle_payment(&payload, &req, None, SettlePhase::AfterHandler)
                .await
                .unwrap()
                .is_success()
        );
        assert_eq!(mock.verifies.load(Ordering::SeqCst), 1);
        assert_eq!(mock.settles.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn find_matching_delegates_to_core() {
        let rs = ResourceServer::new(mock_ok());
        let payload = sample_payload();
        let available = [payload.accepted.clone()];
        assert!(
            rs.find_matching_requirements(&available, &payload)
                .is_some()
        );
    }

    struct AbortBeforeVerify;
    impl ResourceServerHooks for AbortBeforeVerify {
        fn before_verify<'a>(
            &'a self,
            _: &PaymentHookContext,
        ) -> impl Future<Output = BeforeOpDecision<VerifyResponse>> + Send + 'a {
            std::future::ready(BeforeOpDecision::Abort {
                reason: "blocked".into(),
                message: "test".into(),
            })
        }
    }

    #[tokio::test]
    async fn before_verify_abort() {
        let rs = ResourceServer::new(mock_ok()).with_hook(AbortBeforeVerify);
        let payload = sample_payload();
        let req = payload.accepted.clone();
        let err = rs.verify_payment(&payload, &req).await.unwrap_err();
        assert!(matches!(err, FacilitatorError::Aborted { reason, .. } if reason == "blocked"));
    }

    struct SkipVerify;
    impl ResourceServerHooks for SkipVerify {
        fn before_verify<'a>(
            &'a self,
            _: &PaymentHookContext,
        ) -> impl Future<Output = BeforeOpDecision<VerifyResponse>> + Send + 'a {
            std::future::ready(BeforeOpDecision::Skip {
                result: VerifyResponse::valid("0xskip"),
            })
        }
    }

    #[tokio::test]
    async fn before_verify_skip_bypasses_facilitator() {
        let mock = mock_ok();
        let rs = ResourceServer::new(Arc::clone(&mock)).with_hook(SkipVerify);
        let payload = sample_payload();
        let req = payload.accepted.clone();
        let out = rs.verify_payment(&payload, &req).await.unwrap();
        assert!(out.response.is_valid());
        assert_eq!(mock.verifies.load(Ordering::SeqCst), 0);
    }

    struct AfterAbort;
    impl ResourceServerHooks for AfterAbort {
        fn after_verify<'a>(
            &'a self,
            _: &VerifyResultContext,
        ) -> impl Future<Output = AfterVerifyDecision> + Send + 'a {
            std::future::ready(AfterVerifyDecision::Abort {
                reason: "post".into(),
                message: "nope".into(),
            })
        }
    }

    struct CancelHook(Arc<AtomicUsize>);
    impl ResourceServerHooks for CancelHook {
        fn on_verified_payment_canceled<'a>(
            &'a self,
            ctx: &'a VerifiedPaymentCanceledContext,
        ) -> impl Future<Output = ()> + Send + 'a {
            assert_eq!(ctx.reason, CancelReason::AfterVerifyAborted);
            self.0.fetch_add(1, Ordering::SeqCst);
            std::future::ready(())
        }
    }

    #[tokio::test]
    async fn after_verify_abort_fires_cancel() {
        let cancels = Arc::new(AtomicUsize::new(0));
        let rs = ResourceServer::new(mock_ok())
            .with_hook(AfterAbort)
            .with_hook(CancelHook(Arc::clone(&cancels)));
        let payload = sample_payload();
        let req = payload.accepted.clone();
        let err = rs.verify_payment(&payload, &req).await.unwrap_err();
        assert!(matches!(err, FacilitatorError::Aborted { reason, .. } if reason == "post"));
        assert_eq!(cancels.load(Ordering::SeqCst), 1);
    }

    struct SkipHandlerHook;
    impl ResourceServerHooks for SkipHandlerHook {
        fn after_verify<'a>(
            &'a self,
            _: &VerifyResultContext,
        ) -> impl Future<Output = AfterVerifyDecision> + Send + 'a {
            std::future::ready(AfterVerifyDecision::SkipHandler {
                response: SkipHandlerDirective::empty().with_body(serde_json::json!({"ok": true})),
            })
        }
    }

    #[tokio::test]
    async fn after_verify_skip_handler() {
        let rs = ResourceServer::new(mock_ok()).with_hook(SkipHandlerHook);
        let payload = sample_payload();
        let req = payload.accepted.clone();
        let out = rs.verify_payment(&payload, &req).await.unwrap();
        assert!(out.skip_handler.is_some());
    }

    struct RecoverVerify;
    impl ResourceServerHooks for RecoverVerify {
        fn on_verify_failure<'a>(
            &'a self,
            _: &PaymentHookContext,
            _: &FacilitatorError,
        ) -> impl Future<Output = FailureRecovery<VerifyResponse>> + Send + 'a {
            std::future::ready(FailureRecovery::Recovered(VerifyResponse::valid("0xrec")))
        }
    }

    #[tokio::test]
    async fn verify_failure_recovers() {
        let mock = Arc::new(MockFacilitator {
            verifies: AtomicUsize::new(0),
            settles: AtomicUsize::new(0),
            fail_verify: true,
            fail_settle: false,
        });
        let rs = ResourceServer::new(mock).with_hook(RecoverVerify);
        let payload = sample_payload();
        let req = payload.accepted.clone();
        assert!(
            rs.verify_payment(&payload, &req)
                .await
                .unwrap()
                .response
                .is_valid()
        );
    }

    struct CancelCountHook(Arc<AtomicUsize>);
    impl ResourceServerHooks for CancelCountHook {
        fn on_verified_payment_canceled<'a>(
            &'a self,
            _: &VerifiedPaymentCanceledContext,
        ) -> impl Future<Output = ()> + Send + 'a {
            self.0.fetch_add(1, Ordering::SeqCst);
            std::future::ready(())
        }
    }

    #[tokio::test]
    async fn cancellation_guard_fires_once() {
        let count = Arc::new(AtomicUsize::new(0));
        let rs = ResourceServer::new(mock_ok()).with_hook(CancelCountHook(Arc::clone(&count)));
        let payload = sample_payload();
        let req = payload.accepted.clone();
        let guard = rs.cancellation_guard(payload, req);
        guard
            .cancel(CancelReason::HandlerFailed, Some("4xx"), Some(400))
            .await;
        guard.cancel(CancelReason::HandlerThrew, None, None).await;
        assert_eq!(count.load(Ordering::SeqCst), 1);
        assert!(guard.has_fired());
    }

    struct RecordSettlePhase {
        before: Arc<Mutex<Vec<SettlePhase>>>,
        after: Arc<Mutex<Vec<SettlePhase>>>,
        failure: Arc<Mutex<Vec<SettlePhase>>>,
    }

    impl ResourceServerHooks for RecordSettlePhase {
        fn before_settle<'a>(
            &'a self,
            ctx: &'a SettleContext,
        ) -> impl Future<Output = BeforeOpDecision<SettleResponse>> + Send + 'a {
            self.before.lock().unwrap().push(ctx.phase);
            std::future::ready(BeforeOpDecision::Continue)
        }

        fn after_settle<'a>(
            &'a self,
            ctx: &'a SettleResultContext,
        ) -> impl Future<Output = ()> + Send + 'a {
            self.after.lock().unwrap().push(ctx.settle.phase);
            std::future::ready(())
        }

        fn on_settle_failure<'a>(
            &'a self,
            ctx: &'a SettleContext,
            _: &'a FacilitatorError,
        ) -> impl Future<Output = FailureRecovery<SettleResponse>> + Send + 'a {
            self.failure.lock().unwrap().push(ctx.phase);
            std::future::ready(FailureRecovery::Propagate)
        }
    }

    #[tokio::test]
    async fn settle_payment_forwards_phase_to_hooks() {
        for phase in [
            SettlePhase::BeforeHandler,
            SettlePhase::AfterHandler,
            SettlePhase::Cancel,
        ] {
            let hook = RecordSettlePhase {
                before: Arc::new(Mutex::new(Vec::new())),
                after: Arc::new(Mutex::new(Vec::new())),
                failure: Arc::new(Mutex::new(Vec::new())),
            };
            let before = Arc::clone(&hook.before);
            let after = Arc::clone(&hook.after);
            let rs = ResourceServer::new(mock_ok()).with_hook(hook);
            let payload = sample_payload();
            let req = payload.accepted.clone();
            assert!(
                rs.settle_payment(&payload, &req, None, phase)
                    .await
                    .unwrap()
                    .is_success()
            );
            assert_eq!(*before.lock().unwrap(), vec![phase]);
            assert_eq!(*after.lock().unwrap(), vec![phase]);
        }
    }

    #[tokio::test]
    async fn settle_failure_hook_receives_phase() {
        let hook = RecordSettlePhase {
            before: Arc::new(Mutex::new(Vec::new())),
            after: Arc::new(Mutex::new(Vec::new())),
            failure: Arc::new(Mutex::new(Vec::new())),
        };
        let failure = Arc::clone(&hook.failure);
        let mock = Arc::new(MockFacilitator {
            verifies: AtomicUsize::new(0),
            settles: AtomicUsize::new(0),
            fail_verify: false,
            fail_settle: true,
        });
        let rs = ResourceServer::new(mock).with_hook(hook);
        let payload = sample_payload();
        let req = payload.accepted.clone();
        let err = rs
            .settle_payment(&payload, &req, None, SettlePhase::Cancel)
            .await
            .unwrap_err();
        assert!(matches!(err, FacilitatorError::Onchain(_)));
        assert_eq!(*failure.lock().unwrap(), vec![SettlePhase::Cancel]);
    }

    fn pending_failure(transaction: &str) -> SettleResponse {
        SettleResponse::Failure {
            reason: crate::error::ErrorReason::SettlementPending,
            message: None,
            payer: None,
            transaction: transaction.into(),
            network: "eip155:8453".into(),
            extensions: Extensions::new(),
            extra: None,
        }
    }

    fn terminal_failure() -> SettleResponse {
        SettleResponse::Failure {
            reason: crate::error::ErrorReason::UnexpectedSettleError,
            message: None,
            payer: None,
            transaction: CompactString::default(),
            network: "eip155:8453".into(),
            extensions: Extensions::new(),
            extra: None,
        }
    }

    fn success_settle(transaction: &str) -> SettleResponse {
        SettleResponse::Success {
            payer: "0xpayer".into(),
            transaction: transaction.into(),
            network: "eip155:8453".into(),
            amount: None,
            extensions: Extensions::new(),
            extra: None,
        }
    }

    struct QueueFacilitator {
        settles: Mutex<std::collections::VecDeque<Result<SettleResponse, FacilitatorError>>>,
        settle_calls: AtomicUsize,
        requests: Mutex<Vec<SettleRequest>>,
    }

    impl QueueFacilitator {
        fn new(responses: Vec<Result<SettleResponse, FacilitatorError>>) -> Arc<Self> {
            Arc::new(Self {
                settles: Mutex::new(responses.into()),
                settle_calls: AtomicUsize::new(0),
                requests: Mutex::new(Vec::new()),
            })
        }
    }

    impl Facilitator for QueueFacilitator {
        fn verify(
            &self,
            _request: VerifyRequest,
        ) -> impl Future<Output = Result<VerifyResponse, FacilitatorError>> + Send {
            std::future::ready(Ok(VerifyResponse::valid("0xpayer")))
        }

        fn settle(
            &self,
            request: SettleRequest,
        ) -> impl Future<Output = Result<SettleResponse, FacilitatorError>> + Send {
            self.settle_calls.fetch_add(1, Ordering::SeqCst);
            self.requests.lock().unwrap().push(request);
            let next = self
                .settles
                .lock()
                .unwrap()
                .pop_front()
                .unwrap_or_else(|| Ok(success_settle("0xfallback")));
            std::future::ready(next)
        }

        fn supported(
            &self,
        ) -> impl Future<Output = Result<SupportedResponse, FacilitatorError>> + Send {
            std::future::ready(Ok(SupportedResponse::default()))
        }
    }

    #[tokio::test]
    async fn settle_does_not_retry_on_success() {
        let mock = QueueFacilitator::new(vec![Ok(success_settle("0x1"))]);
        let rs = ResourceServer::new(Arc::clone(&mock));
        let payload = sample_payload();
        let req = payload.accepted.clone();
        let result = rs
            .settle_payment(&payload, &req, None, SettlePhase::AfterHandler)
            .await
            .unwrap();
        assert!(result.is_success());
        assert_eq!(mock.settle_calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn settle_does_not_retry_on_terminal_failure() {
        let mock = QueueFacilitator::new(vec![Ok(terminal_failure())]);
        let rs = ResourceServer::new(Arc::clone(&mock));
        let payload = sample_payload();
        let req = payload.accepted.clone();
        let result = rs
            .settle_payment(&payload, &req, None, SettlePhase::AfterHandler)
            .await
            .unwrap();
        assert!(!result.is_success());
        assert_eq!(mock.settle_calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn settle_does_not_retry_on_facilitator_error() {
        let mock =
            QueueFacilitator::new(vec![Err(FacilitatorError::Onchain("rpc timeout".into()))]);
        let rs = ResourceServer::new(Arc::clone(&mock));
        let payload = sample_payload();
        let req = payload.accepted.clone();
        let err = rs
            .settle_payment(&payload, &req, None, SettlePhase::AfterHandler)
            .await
            .unwrap_err();
        assert!(matches!(err, FacilitatorError::Onchain(_)));
        assert_eq!(mock.settle_calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn settle_retries_once_on_pending_then_success() {
        let mock = QueueFacilitator::new(vec![
            Ok(pending_failure("0xpending")),
            Ok(success_settle("0xconfirmed")),
        ]);
        let rs = ResourceServer::new(Arc::clone(&mock));
        let payload = sample_payload();
        let req = payload.accepted.clone();
        let result = rs
            .settle_payment(&payload, &req, None, SettlePhase::AfterHandler)
            .await
            .unwrap();
        assert!(matches!(
            result,
            SettleResponse::Success { ref transaction, .. } if transaction == "0xconfirmed"
        ));
        assert_eq!(mock.settle_calls.load(Ordering::SeqCst), 2);
        let requests = mock.requests.lock().unwrap();
        assert_eq!(
            requests[0].clone().into_json(),
            requests[1].clone().into_json()
        );
        drop(requests);
    }

    #[tokio::test]
    async fn settle_does_not_retry_pending_with_empty_transaction() {
        let mock = QueueFacilitator::new(vec![Ok(pending_failure(""))]);
        let rs = ResourceServer::new(Arc::clone(&mock));
        let payload = sample_payload();
        let req = payload.accepted.clone();
        let result = rs
            .settle_payment(&payload, &req, None, SettlePhase::AfterHandler)
            .await
            .unwrap();
        assert!(!result.is_success());
        assert_eq!(mock.settle_calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn settle_caps_retry_when_second_attempt_still_pending() {
        let mock = QueueFacilitator::new(vec![
            Ok(pending_failure("0xfirst")),
            Ok(pending_failure("0xsecond")),
        ]);
        let rs = ResourceServer::new(Arc::clone(&mock));
        let payload = sample_payload();
        let req = payload.accepted.clone();
        let result = rs
            .settle_payment(&payload, &req, None, SettlePhase::AfterHandler)
            .await
            .unwrap();
        assert!(matches!(
            result,
            SettleResponse::Failure { ref transaction, .. } if transaction == "0xsecond"
        ));
        assert_eq!(mock.settle_calls.load(Ordering::SeqCst), 2);
    }
    #[cfg(feature = "metrics")]
    mod metrics_recording {
        use std::future::Future;
        use std::sync::Arc;
        use std::sync::atomic::{AtomicU64, Ordering};

        use metrics::{
            Counter, CounterFn, Gauge, Histogram, HistogramFn, Key, KeyName, Metadata, Recorder,
            SharedString, Unit,
        };

        use super::{
            AbortBeforeVerify, Facilitator, FacilitatorError, MockFacilitator, RecoverVerify,
            ResourceServer, SettlePhase, SkipVerify, SupportedResponse, VerifyRequest,
            VerifyResponse, mock_ok, sample_payload,
        };
        use crate::error::ErrorReason;
        use crate::metrics::{
            FACILITATOR_SETTLE_DURATION_SECONDS, FACILITATOR_SETTLE_TOTAL,
            FACILITATOR_VERIFY_DURATION_SECONDS, FACILITATOR_VERIFY_TOTAL,
        };
        use crate::wire::{Extensions, SettleRequest, SettleResponse};

        struct Count(AtomicU64);

        impl Count {
            fn arc() -> Arc<Self> {
                Arc::new(Self(AtomicU64::new(0)))
            }

            fn get(handle: &Arc<Self>) -> u64 {
                handle.0.load(Ordering::Relaxed)
            }
        }

        impl CounterFn for Count {
            fn increment(&self, value: u64) {
                self.0.fetch_add(value, Ordering::Relaxed);
            }

            fn absolute(&self, value: u64) {
                self.0.store(value, Ordering::Relaxed);
            }
        }

        impl HistogramFn for Count {
            fn record(&self, _value: f64) {
                self.0.fetch_add(1, Ordering::Relaxed);
            }
        }

        struct Capture {
            verify_valid: Arc<Count>,
            verify_invalid: Arc<Count>,
            verify_error: Arc<Count>,
            verify_duration: Arc<Count>,
            settle_success: Arc<Count>,
            settle_failure: Arc<Count>,
            settle_error: Arc<Count>,
            settle_duration: Arc<Count>,
        }

        impl Capture {
            fn new() -> Self {
                Self {
                    verify_valid: Count::arc(),
                    verify_invalid: Count::arc(),
                    verify_error: Count::arc(),
                    verify_duration: Count::arc(),
                    settle_success: Count::arc(),
                    settle_failure: Count::arc(),
                    settle_error: Count::arc(),
                    settle_duration: Count::arc(),
                }
            }

            fn counter_handle(&self, name: &str, result: &str) -> Option<Arc<Count>> {
                Some(Arc::clone(match (name, result) {
                    (FACILITATOR_VERIFY_TOTAL, "valid") => &self.verify_valid,
                    (FACILITATOR_VERIFY_TOTAL, "invalid") => &self.verify_invalid,
                    (FACILITATOR_VERIFY_TOTAL, "error") => &self.verify_error,
                    (FACILITATOR_SETTLE_TOTAL, "success") => &self.settle_success,
                    (FACILITATOR_SETTLE_TOTAL, "failure") => &self.settle_failure,
                    (FACILITATOR_SETTLE_TOTAL, "error") => &self.settle_error,
                    _ => return None,
                }))
            }

            fn histogram_handle(&self, name: &str) -> Option<Arc<Count>> {
                Some(Arc::clone(match name {
                    FACILITATOR_VERIFY_DURATION_SECONDS => &self.verify_duration,
                    FACILITATOR_SETTLE_DURATION_SECONDS => &self.settle_duration,
                    _ => return None,
                }))
            }
        }

        impl Recorder for Capture {
            fn describe_counter(&self, _: KeyName, _: Option<Unit>, _: SharedString) {}
            fn describe_gauge(&self, _: KeyName, _: Option<Unit>, _: SharedString) {}
            fn describe_histogram(&self, _: KeyName, _: Option<Unit>, _: SharedString) {}

            fn register_counter(&self, key: &Key, _: &Metadata<'_>) -> Counter {
                let result = key
                    .labels()
                    .find(|label| label.key() == "result")
                    .map_or("", |label| label.value());
                self.counter_handle(key.name(), result)
                    .map_or_else(Counter::noop, Counter::from_arc)
            }

            fn register_gauge(&self, _: &Key, _: &Metadata<'_>) -> Gauge {
                Gauge::noop()
            }

            fn register_histogram(&self, key: &Key, _: &Metadata<'_>) -> Histogram {
                self.histogram_handle(key.name())
                    .map_or_else(Histogram::noop, Histogram::from_arc)
            }
        }

        struct InvalidVerify;
        impl Facilitator for InvalidVerify {
            fn verify(
                &self,
                _request: VerifyRequest,
            ) -> impl Future<Output = Result<VerifyResponse, FacilitatorError>> + Send {
                std::future::ready(Ok(VerifyResponse::invalid(
                    None,
                    ErrorReason::InvalidPayload,
                )))
            }
            fn settle(
                &self,
                _request: SettleRequest,
            ) -> impl Future<Output = Result<SettleResponse, FacilitatorError>> + Send {
                std::future::ready(Err(FacilitatorError::Onchain("unreachable".into())))
            }
            fn supported(
                &self,
            ) -> impl Future<Output = Result<SupportedResponse, FacilitatorError>> + Send
            {
                std::future::ready(Ok(SupportedResponse::default()))
            }
        }

        struct FailedSettle;
        impl Facilitator for FailedSettle {
            fn verify(
                &self,
                _request: VerifyRequest,
            ) -> impl Future<Output = Result<VerifyResponse, FacilitatorError>> + Send {
                std::future::ready(Err(FacilitatorError::Onchain("unreachable".into())))
            }
            fn settle(
                &self,
                _request: SettleRequest,
            ) -> impl Future<Output = Result<SettleResponse, FacilitatorError>> + Send {
                std::future::ready(Ok(SettleResponse::Failure {
                    reason: ErrorReason::UnexpectedSettleError,
                    message: None,
                    payer: None,
                    transaction: compact_str::CompactString::default(),
                    network: "eip155:1".into(),
                    extensions: Extensions::new(),
                    extra: None,
                }))
            }
            fn supported(
                &self,
            ) -> impl Future<Output = Result<SupportedResponse, FacilitatorError>> + Send
            {
                std::future::ready(Ok(SupportedResponse::default()))
            }
        }

        #[tokio::test]
        async fn resource_server_verify_records_without_hooked_wrapper() {
            let capture = Capture::new();
            let _guard = metrics::set_default_local_recorder(&capture);
            let payload = sample_payload();
            let req = payload.accepted.clone();

            let ok = ResourceServer::new(mock_ok());
            assert!(
                ok.verify_payment(&payload, &req)
                    .await
                    .unwrap()
                    .response
                    .is_valid(),
                "ok verify"
            );

            let recovered = ResourceServer::new(Arc::new(MockFacilitator {
                verifies: std::sync::atomic::AtomicUsize::new(0),
                settles: std::sync::atomic::AtomicUsize::new(0),
                fail_verify: true,
                fail_settle: false,
            }))
            .with_hook(RecoverVerify);
            assert!(
                recovered
                    .verify_payment(&payload, &req)
                    .await
                    .unwrap()
                    .response
                    .is_valid(),
                "recovered verify"
            );

            let invalid = ResourceServer::new(Arc::new(InvalidVerify));
            assert!(
                !invalid
                    .verify_payment(&payload, &req)
                    .await
                    .unwrap()
                    .response
                    .is_valid(),
                "invalid verify"
            );

            let failing = ResourceServer::new(Arc::new(MockFacilitator {
                verifies: std::sync::atomic::AtomicUsize::new(0),
                settles: std::sync::atomic::AtomicUsize::new(0),
                fail_verify: true,
                fail_settle: false,
            }));
            assert!(
                failing.verify_payment(&payload, &req).await.is_err(),
                "failing verify"
            );

            assert_eq!(Count::get(&capture.verify_valid), 2, "valid+recovered");
            assert_eq!(Count::get(&capture.verify_invalid), 1, "invalid");
            assert_eq!(Count::get(&capture.verify_error), 1, "error");
            assert_eq!(Count::get(&capture.verify_duration), 4, "verify samples");
        }

        #[tokio::test]
        async fn resource_server_skip_and_abort_do_not_record_verify() {
            let capture = Capture::new();
            let _guard = metrics::set_default_local_recorder(&capture);
            let payload = sample_payload();
            let req = payload.accepted.clone();

            let skipped = ResourceServer::new(mock_ok()).with_hook(SkipVerify);
            assert!(
                skipped
                    .verify_payment(&payload, &req)
                    .await
                    .unwrap()
                    .response
                    .is_valid(),
                "skipped verify"
            );

            let aborted = ResourceServer::new(mock_ok()).with_hook(AbortBeforeVerify);
            assert!(
                aborted.verify_payment(&payload, &req).await.is_err(),
                "aborted verify"
            );

            assert_eq!(Count::get(&capture.verify_valid), 0, "skip is not a verify");
            assert_eq!(
                Count::get(&capture.verify_error),
                0,
                "abort is not a verify"
            );
            assert_eq!(Count::get(&capture.verify_duration), 0, "no duration");
        }

        #[tokio::test]
        async fn resource_server_settle_records_without_hooked_wrapper() {
            let capture = Capture::new();
            let _guard = metrics::set_default_local_recorder(&capture);
            let payload = sample_payload();
            let req = payload.accepted.clone();

            let ok = ResourceServer::new(mock_ok());
            assert!(
                ok.settle_payment(&payload, &req, None, SettlePhase::AfterHandler)
                    .await
                    .unwrap()
                    .is_success(),
                "ok settle"
            );

            let failed = ResourceServer::new(Arc::new(FailedSettle));
            assert!(
                !failed
                    .settle_payment(&payload, &req, None, SettlePhase::AfterHandler)
                    .await
                    .unwrap()
                    .is_success(),
                "failed settle"
            );

            let failing = ResourceServer::new(Arc::new(MockFacilitator {
                verifies: std::sync::atomic::AtomicUsize::new(0),
                settles: std::sync::atomic::AtomicUsize::new(0),
                fail_verify: false,
                fail_settle: true,
            }));
            assert!(
                failing
                    .settle_payment(&payload, &req, None, SettlePhase::AfterHandler)
                    .await
                    .is_err(),
                "failing settle"
            );

            assert_eq!(Count::get(&capture.settle_success), 1, "success");
            assert_eq!(Count::get(&capture.settle_failure), 1, "failure");
            assert_eq!(Count::get(&capture.settle_error), 1, "error");
            assert_eq!(Count::get(&capture.settle_duration), 3, "settle samples");
        }
    }
}
