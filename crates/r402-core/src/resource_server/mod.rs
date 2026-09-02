//! Resource-server orchestration shared by HTTP and MCP transports.
//!
//! Mirrors the official Go/TS `X402ResourceServer` **V2** surface used by
//! transports: match requirements, verify, settle, and lifecycle hooks
//! including verified-payment cancellation.
//!
//! This type does **not** own pricing or route maps. Registered
//! [`SchemeNetworkServer`] adapters resolve payment flow; verify and settle
//! still go through a [`Facilitator`].

mod hooks;
mod payment_flow;
mod scheme;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

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
pub use scheme::{DynSchemeNetworkServer, SchemeNetworkServer, SchemePaymentRequiredContext};
use serde::Serialize;

use crate::chain::{ChainId, ChainIdPattern};
use crate::error::{FacilitatorError, VerificationError};
use crate::facilitator::FailureRecovery;
use crate::facilitator::{DynFacilitator, Facilitator};
use crate::wire::{
    Extensions, PaymentRequirements, SettleRequest, SettleResponse, TypedVerifyRequest, V2,
    VerifyRequest, VerifyResponse,
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

/// Server-side payment orchestrator (official `X402ResourceServer`, V2-only).
pub struct ResourceServer {
    facilitator: Arc<dyn DynFacilitator>,
    hooks: Vec<Arc<dyn DynResourceServerHooks>>,
    schemes: Vec<(ChainIdPattern, Arc<dyn DynSchemeNetworkServer>)>,
}

impl Clone for ResourceServer {
    fn clone(&self) -> Self {
        Self {
            facilitator: Arc::clone(&self.facilitator),
            hooks: self.hooks.clone(),
            schemes: self.schemes.clone(),
        }
    }
}

impl std::fmt::Debug for ResourceServer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ResourceServer")
            .field("hooks", &self.hooks.len())
            .field("schemes", &self.schemes.len())
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
            schemes: Vec::new(),
        }
    }

    /// Creates from an already-erased facilitator handle.
    #[must_use]
    pub fn from_dyn(facilitator: Arc<dyn DynFacilitator>) -> Self {
        Self {
            facilitator,
            hooks: Vec::new(),
            schemes: Vec::new(),
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

    /// Registers a scheme/network adapter. Returns `self` for builder chaining.
    #[must_use]
    pub fn with_scheme(
        mut self,
        network: ChainIdPattern,
        scheme: impl SchemeNetworkServer + 'static,
    ) -> Self {
        self.register_scheme(network, scheme);
        self
    }

    /// Registers a scheme/network adapter after construction.
    pub fn register_scheme(
        &mut self,
        network: ChainIdPattern,
        scheme: impl SchemeNetworkServer + 'static,
    ) {
        self.schemes.push((network, Arc::new(scheme)));
    }

    /// Registered adapter for `scheme` on `network`, if any.
    ///
    /// Exact CAIP-2 registrations win over wildcard/set patterns.
    #[must_use]
    pub fn registered_scheme(
        &self,
        scheme: &str,
        network: &ChainId,
    ) -> Option<&dyn DynSchemeNetworkServer> {
        let mut patterned: Option<&dyn DynSchemeNetworkServer> = None;
        for (pattern, server) in &self.schemes {
            if server.scheme() != scheme || !pattern.matches(network) {
                continue;
            }
            if matches!(pattern, ChainIdPattern::Exact { .. }) {
                return Some(server.as_ref());
            }
            if patterned.is_none() {
                patterned = Some(server.as_ref());
            }
        }
        patterned
    }

    /// Resolves the payment-flow name for this accept from the scheme table.
    ///
    /// # Errors
    ///
    /// [`PaymentFlowError::UnregisteredScheme`] when no adapter matches this
    /// accept, or ATM/flow errors from [`resolve_payment_flow`].
    pub fn get_payment_flow(
        &self,
        requirements: &PaymentRequirements,
    ) -> Result<PaymentFlowName, PaymentFlowError> {
        let Some(scheme) =
            self.registered_scheme(requirements.scheme.as_str(), &requirements.network)
        else {
            return Err(PaymentFlowError::UnregisteredScheme {
                scheme: requirements.scheme.to_string(),
                network: requirements.network.to_string(),
            });
        };
        let resolved = resolve_payment_flow(
            &PaymentFlowScheme {
                scheme: scheme.scheme(),
                default_asset_transfer_method: scheme.default_asset_transfer_method(),
                payment_flows: scheme.payment_flows(),
            },
            requirements,
        )?;
        Ok(resolved.payment_flow)
    }

    /// Official `FindMatchingRequirements`.
    ///
    /// Scheme-declared [`SchemeNetworkServer::dynamic_extra_fields`] are
    /// omitted from the extra-subset comparison.
    #[must_use]
    pub fn find_matching_requirements<'a>(
        &self,
        available: &'a [PaymentRequirements],
        payload: &WirePaymentPayload,
    ) -> Option<&'a PaymentRequirements> {
        available.iter().find(|req| {
            let dynamic = self
                .registered_scheme(req.scheme.as_str(), &req.network)
                .map_or(&[][..], DynSchemeNetworkServer::dynamic_extra_fields);
            req.matches_payload_accepted_with_dynamic(&payload.accepted, dynamic)
        })
    }

    /// Official `VerifyPayment`: before hooks → facilitator → after hooks.
    ///
    /// When `schemes` is empty, facilitator verify always runs. When schemes
    /// are registered and the resolved flow has `!verify_before_handler`,
    /// returns Valid without the facilitator and without `after_verify`.
    ///
    /// # Errors
    ///
    /// Propagates facilitator errors (unless recovered), before-hook aborts,
    /// after-verify aborts (`FacilitatorError::Aborted`), and payment-flow
    /// resolution failures.
    pub async fn verify_payment(
        &self,
        payload: &WirePaymentPayload,
        requirements: &PaymentRequirements,
    ) -> Result<VerifyPaymentOutcome, FacilitatorError> {
        let payment = PaymentHookContext {
            payload: payload.clone(),
            requirements: requirements.clone(),
        };

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
        } else if self.schemes.is_empty() {
            self.call_verify_with_failure_hooks(&payment, payload, requirements)
                .await?
        } else {
            let flow = self.get_payment_flow(requirements).map_err(|err| {
                FacilitatorError::Verification(VerificationError::InvalidFormat(err.to_string()))
            })?;
            if resolve_payment_flow_phases(flow).verify_before_handler {
                self.call_verify_with_failure_hooks(&payment, payload, requirements)
                    .await?
            } else {
                return Ok(VerifyPaymentOutcome {
                    response: VerifyResponse::valid(""),
                    skip_handler: None,
                });
            }
        };

        self.run_after_verify(payload, requirements, payment, response)
            .await
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
            settled_phases: Vec::new(),
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
            settled_phases: Vec::new(),
            before_handler: None,
        }
    }

    async fn run_after_verify(
        &self,
        payload: &WirePaymentPayload,
        requirements: &PaymentRequirements,
        payment: PaymentHookContext,
        response: VerifyResponse,
    ) -> Result<VerifyPaymentOutcome, FacilitatorError> {
        if !response.is_valid() {
            return Ok(VerifyPaymentOutcome {
                response,
                skip_handler: None,
            });
        }

        let result_ctx = VerifyResultContext {
            payment,
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
                    skip_handler = Some(directive);
                }
            }
        }

        Ok(VerifyPaymentOutcome {
            response,
            skip_handler,
        })
    }

    async fn settle_canceled_payment(
        &self,
        ctx: &VerifiedPaymentCanceledContext,
    ) -> Option<SettleResponse> {
        if !ctx.settled_phases.contains(&SettlePhase::BeforeHandler) {
            return None;
        }
        let scheme = self.registered_scheme(
            ctx.payment.requirements.scheme.as_str(),
            &ctx.payment.requirements.network,
        )?;
        let cancel_requirements = scheme.settle_on_cancel(ctx).await?;
        match self
            .settle_payment(
                &ctx.payment.payload,
                &cancel_requirements,
                None,
                SettlePhase::Cancel,
            )
            .await
        {
            Ok(response) => Some(response),
            Err(error) => Some(SettleResponse::from_facilitator_error(
                &error,
                ctx.payment.requirements.network.to_string(),
                "",
            )),
        }
    }

    async fn call_verify_with_failure_hooks(
        &self,
        payment: &PaymentHookContext,
        payload: &WirePaymentPayload,
        requirements: &PaymentRequirements,
    ) -> Result<VerifyResponse, FacilitatorError> {
        let request = build_verify_request(payload, requirements)?;
        match DynFacilitator::verify(self.facilitator.as_ref(), request).await {
            Ok(r) if r.is_valid() => Ok(r),
            Ok(invalid) => {
                let error = facilitator_error_from_invalid(&invalid);
                self.recover_verify(payment, error).await.or(Ok(invalid))
            }
            Err(error) => self.recover_verify(payment, error).await,
        }
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
        match self.settle_with_pending_retry(request).await {
            Ok(r) => Ok(r),
            Err(error) => self.recover_settle(settle, error).await,
        }
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

/// Successful settle captured for a later phase (echo / cancel).
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct CompletedSettlement {
    /// Settle invocation that produced `result`.
    pub phase: SettlePhase,
    /// Payment flow in effect when this settle ran.
    pub flow: PaymentFlowName,
    /// Facilitator settle response.
    pub result: SettleResponse,
    /// Requirements used for this settle.
    pub requirements: PaymentRequirements,
}

impl CompletedSettlement {
    /// Constructs a completed-settlement record.
    #[must_use]
    pub const fn new(
        phase: SettlePhase,
        flow: PaymentFlowName,
        result: SettleResponse,
        requirements: PaymentRequirements,
    ) -> Self {
        Self {
            phase,
            flow,
            result,
            requirements,
        }
    }
}

/// Ensures `on_verified_payment_canceled` runs at most once for a payment.
#[derive(Debug)]
pub struct CancellationGuard {
    server: ResourceServer,
    payload: WirePaymentPayload,
    requirements: PaymentRequirements,
    fired: AtomicBool,
    settled_phases: Vec<SettlePhase>,
    before_handler: Option<CompletedSettlement>,
}

impl CancellationGuard {
    /// Records settle phases already completed before the handler.
    #[must_use]
    pub fn with_settled_phases(mut self, phases: impl IntoIterator<Item = SettlePhase>) -> Self {
        self.settled_phases = phases.into_iter().collect();
        self
    }

    /// Stores the before-handler settle receipt for failure-path echo.
    #[must_use]
    pub fn with_before_handler(mut self, settlement: CompletedSettlement) -> Self {
        self.before_handler = Some(settlement);
        self
    }

    /// Settle phases recorded on this guard.
    #[must_use]
    pub const fn settled_phases(&self) -> &[SettlePhase] {
        self.settled_phases.as_slice()
    }

    /// Before-handler settle receipt, when one was stored.
    #[must_use]
    pub const fn before_handler(&self) -> Option<&CompletedSettlement> {
        self.before_handler.as_ref()
    }

    /// Fires cancel hooks if not already fired.
    ///
    /// When [`SettlePhase::BeforeHandler`] is in [`Self::settled_phases`] and
    /// the matched scheme's `settle_on_cancel` returns requirements, settles
    /// at [`SettlePhase::Cancel`].
    pub async fn cancel(
        &self,
        reason: CancelReason,
        error: Option<&str>,
        response_status: Option<u16>,
    ) -> Option<SettleResponse> {
        if self
            .fired
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_err()
        {
            return None;
        }
        let ctx = VerifiedPaymentCanceledContext {
            payment: PaymentHookContext {
                payload: self.payload.clone(),
                requirements: self.requirements.clone(),
            },
            reason,
            error: error.map(str::to_owned),
            response_status,
            settled_phases: self.settled_phases.clone(),
        };
        for hook in &self.server.hooks {
            hook.on_verified_payment_canceled(&ctx).await;
        }
        self.server.settle_canceled_payment(&ctx).await
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
    use std::collections::HashMap;
    use std::future::Future;
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use compact_str::CompactString;

    use super::*;
    use crate::chain::ChainIdPattern;
    use crate::wire::{Extensions, SupportedResponse};

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
        let (first, second) = {
            let requests = mock.requests.lock().unwrap();
            (
                requests[0].clone().into_json(),
                requests[1].clone().into_json(),
            )
        };
        assert_eq!(first, second);
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

    struct MockScheme {
        name: &'static str,
        default_atm: &'static str,
        flows: HashMap<String, PaymentFlowConfig>,
        dynamic: &'static [&'static str],
        cancel_requirements: Option<PaymentRequirements>,
    }

    impl MockScheme {
        fn with_flow(flow: PaymentFlowName) -> Self {
            let mut flows = HashMap::new();
            flows.insert(
                SDK_DEFAULT_ASSET_TRANSFER_METHOD.into(),
                PaymentFlowConfig::new(vec![flow], flow),
            );
            Self {
                name: "exact",
                default_atm: SDK_DEFAULT_ASSET_TRANSFER_METHOD,
                flows,
                dynamic: &[],
                cancel_requirements: None,
            }
        }

        fn authorization() -> Self {
            Self::with_flow(PaymentFlowName::Authorization)
        }

        fn upfront() -> Self {
            Self::with_flow(PaymentFlowName::Upfront)
        }

        fn escrow() -> Self {
            Self::with_flow(PaymentFlowName::Escrow)
        }

        fn auth_and_upfront() -> Self {
            let mut flows = HashMap::new();
            flows.insert(
                SDK_DEFAULT_ASSET_TRANSFER_METHOD.into(),
                PaymentFlowConfig::new(
                    vec![PaymentFlowName::Authorization, PaymentFlowName::Upfront],
                    PaymentFlowName::Authorization,
                ),
            );
            Self {
                name: "exact",
                default_atm: SDK_DEFAULT_ASSET_TRANSFER_METHOD,
                flows,
                dynamic: &[],
                cancel_requirements: None,
            }
        }
    }

    impl SchemeNetworkServer for MockScheme {
        fn scheme(&self) -> &str {
            self.name
        }

        fn default_asset_transfer_method(&self) -> &str {
            self.default_atm
        }

        fn payment_flows(&self) -> &HashMap<String, PaymentFlowConfig> {
            &self.flows
        }

        fn dynamic_extra_fields(&self) -> &[&str] {
            self.dynamic
        }

        fn settle_on_cancel<'a>(
            &'a self,
            _: &'a VerifiedPaymentCanceledContext,
        ) -> impl Future<Output = Option<PaymentRequirements>> + Send + 'a {
            std::future::ready(self.cancel_requirements.clone())
        }
    }

    struct AfterVerifyCount(Arc<AtomicUsize>);
    impl ResourceServerHooks for AfterVerifyCount {
        fn after_verify<'a>(
            &'a self,
            _: &VerifyResultContext,
        ) -> impl Future<Output = AfterVerifyDecision> + Send + 'a {
            self.0.fetch_add(1, Ordering::SeqCst);
            std::future::ready(AfterVerifyDecision::Continue)
        }
    }

    struct RecordCancelPhases(Arc<Mutex<Vec<SettlePhase>>>);
    impl ResourceServerHooks for RecordCancelPhases {
        fn on_verified_payment_canceled<'a>(
            &'a self,
            ctx: &'a VerifiedPaymentCanceledContext,
        ) -> impl Future<Output = ()> + Send + 'a {
            self.0
                .lock()
                .unwrap()
                .extend(ctx.settled_phases.iter().copied());
            std::future::ready(())
        }
    }

    fn eip155_wildcard() -> ChainIdPattern {
        ChainIdPattern::wildcard("eip155")
    }

    #[test]
    fn get_payment_flow_errors_when_table_empty() {
        let rs = ResourceServer::new(mock_ok());
        let req = sample_payload().accepted;
        let err = rs.get_payment_flow(&req).unwrap_err();
        assert!(matches!(
            err,
            PaymentFlowError::UnregisteredScheme { ref scheme, ref network }
                if scheme == "exact" && network == "eip155:1"
        ));
        assert!(
            err.to_string().contains(
                "No server implementation registered for scheme: exact, network: eip155:1"
            )
        );
    }

    #[test]
    fn get_payment_flow_errors_if_unregistered_on_non_empty_table() {
        let rs = ResourceServer::new(mock_ok()).with_scheme(
            ChainIdPattern::exact("eip155", "8453"),
            MockScheme::authorization(),
        );
        let err = rs.get_payment_flow(&sample_payload().accepted).unwrap_err();
        assert!(matches!(err, PaymentFlowError::UnregisteredScheme { .. }));
    }

    #[test]
    fn get_payment_flow_resolves_registered_scheme() {
        let rs =
            ResourceServer::new(mock_ok()).with_scheme(eip155_wildcard(), MockScheme::upfront());
        assert_eq!(
            rs.get_payment_flow(&sample_payload().accepted).unwrap(),
            PaymentFlowName::Upfront
        );
    }

    #[tokio::test]
    async fn verify_unregistered_scheme_on_non_empty_table_errors() {
        let mock = mock_ok();
        let rs = ResourceServer::new(Arc::clone(&mock)).with_scheme(
            ChainIdPattern::exact("solana", "mainnet"),
            MockScheme::authorization(),
        );
        let payload = sample_payload();
        let req = payload.accepted.clone();
        let err = rs.verify_payment(&payload, &req).await.unwrap_err();
        assert!(matches!(
            err,
            FacilitatorError::Verification(VerificationError::InvalidFormat(ref msg))
                if msg.contains("No server implementation registered")
        ));
        assert_eq!(mock.verifies.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn verify_skips_facilitator_and_after_verify_when_not_verify_before_handler() {
        let mock = mock_ok();
        let after = Arc::new(AtomicUsize::new(0));
        let rs = ResourceServer::new(Arc::clone(&mock))
            .with_scheme(eip155_wildcard(), MockScheme::upfront())
            .with_hook(AfterVerifyCount(Arc::clone(&after)))
            .with_hook(AfterAbort);
        let payload = sample_payload();
        let req = payload.accepted.clone();
        let out = rs.verify_payment(&payload, &req).await.unwrap();
        assert!(out.response.is_valid());
        assert!(out.skip_handler.is_none());
        assert_eq!(mock.verifies.load(Ordering::SeqCst), 0);
        assert_eq!(after.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn verify_escrow_also_skips_facilitator() {
        let mock = mock_ok();
        let rs = ResourceServer::new(Arc::clone(&mock))
            .with_scheme(eip155_wildcard(), MockScheme::escrow());
        let payload = sample_payload();
        let req = payload.accepted.clone();
        assert!(
            rs.verify_payment(&payload, &req)
                .await
                .unwrap()
                .response
                .is_valid()
        );
        assert_eq!(mock.verifies.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn verify_authorization_scheme_still_calls_facilitator() {
        let mock = mock_ok();
        let after = Arc::new(AtomicUsize::new(0));
        let rs = ResourceServer::new(Arc::clone(&mock))
            .with_scheme(eip155_wildcard(), MockScheme::authorization())
            .with_hook(AfterVerifyCount(Arc::clone(&after)));
        let payload = sample_payload();
        let req = payload.accepted.clone();
        assert!(
            rs.verify_payment(&payload, &req)
                .await
                .unwrap()
                .response
                .is_valid()
        );
        assert_eq!(mock.verifies.load(Ordering::SeqCst), 1);
        assert_eq!(after.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn before_verify_skip_still_runs_after_verify_with_upfront_scheme() {
        let mock = mock_ok();
        let rs = ResourceServer::new(Arc::clone(&mock))
            .with_scheme(eip155_wildcard(), MockScheme::upfront())
            .with_hook(SkipVerify)
            .with_hook(AfterAbort);
        let payload = sample_payload();
        let req = payload.accepted.clone();
        let err = rs.verify_payment(&payload, &req).await.unwrap_err();
        assert!(matches!(err, FacilitatorError::Aborted { reason, .. } if reason == "post"));
        assert_eq!(mock.verifies.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn before_verify_abort_runs_before_scheme_lookup() {
        let mock = mock_ok();
        let rs = ResourceServer::new(Arc::clone(&mock))
            .with_scheme(
                ChainIdPattern::exact("solana", "mainnet"),
                MockScheme::authorization(),
            )
            .with_hook(AbortBeforeVerify);
        let payload = sample_payload();
        let req = payload.accepted.clone();
        let err = rs.verify_payment(&payload, &req).await.unwrap_err();
        assert!(matches!(err, FacilitatorError::Aborted { reason, .. } if reason == "blocked"));
        assert_eq!(mock.verifies.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn verify_unsupported_payment_flow_errors() {
        let mock = mock_ok();
        let rs = ResourceServer::new(Arc::clone(&mock))
            .with_scheme(eip155_wildcard(), MockScheme::auth_and_upfront());
        let payload = sample_payload();
        let mut req = payload.accepted.clone();
        req.extra = Some(serde_json::json!({ "paymentFlow": "escrow" }));
        let err = rs.verify_payment(&payload, &req).await.unwrap_err();
        assert!(matches!(
            err,
            FacilitatorError::Verification(VerificationError::InvalidFormat(ref msg))
                if msg.contains("does not support paymentFlow \"escrow\"")
        ));
        assert_eq!(mock.verifies.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn exact_registration_wins_over_wildcard() {
        let mock = mock_ok();
        let rs = ResourceServer::new(Arc::clone(&mock))
            .with_scheme(eip155_wildcard(), MockScheme::upfront())
            .with_scheme(
                ChainIdPattern::exact("eip155", "1"),
                MockScheme::authorization(),
            );
        let payload = sample_payload();
        let req = payload.accepted.clone();
        rs.verify_payment(&payload, &req).await.unwrap();
        assert_eq!(mock.verifies.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn find_matching_omits_scheme_dynamic_extra_fields() {
        let mut required = sample_payload().accepted;
        required.extra = Some(serde_json::json!({
            "feePayer": "FeePayer111111111111111111111111111111111",
            "recentBlockhash": "fresh"
        }));
        let mut accepted = required.clone();
        accepted.extra = Some(serde_json::json!({
            "feePayer": "FeePayer111111111111111111111111111111111",
            "recentBlockhash": "stale"
        }));
        let payload = WirePaymentPayload::new(accepted, serde_json::json!({ "k": 1 }));
        let available = [required];

        let empty = ResourceServer::new(mock_ok());
        assert!(
            empty
                .find_matching_requirements(&available, &payload)
                .is_none()
        );

        let mut scheme = MockScheme::authorization();
        scheme.dynamic = &["recentBlockhash"];
        let rs = ResourceServer::new(mock_ok()).with_scheme(eip155_wildcard(), scheme);
        assert!(
            rs.find_matching_requirements(&available, &payload)
                .is_some()
        );
    }

    #[tokio::test]
    async fn cancel_settles_when_before_handler_and_settle_on_cancel() {
        let mock = mock_ok();
        let payload = sample_payload();
        let req = payload.accepted.clone();
        let mut scheme = MockScheme::escrow();
        scheme.cancel_requirements = Some(req.clone());
        let phases = Arc::new(Mutex::new(Vec::new()));
        let rs = ResourceServer::new(Arc::clone(&mock))
            .with_scheme(eip155_wildcard(), scheme)
            .with_hook(RecordCancelPhases(Arc::clone(&phases)));
        let guard = rs
            .cancellation_guard(payload, req)
            .with_settled_phases([SettlePhase::BeforeHandler]);
        let receipt = guard
            .cancel(CancelReason::HandlerFailed, Some("4xx"), Some(400))
            .await;
        assert!(receipt.unwrap().is_success());
        assert_eq!(mock.settles.load(Ordering::SeqCst), 1);
        assert_eq!(*phases.lock().unwrap(), vec![SettlePhase::BeforeHandler]);
        assert!(guard.has_fired());
        assert!(
            guard
                .cancel(CancelReason::HandlerThrew, None, None)
                .await
                .is_none()
        );
        assert_eq!(mock.settles.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn cancel_skips_settle_without_before_handler_phase() {
        let mock = mock_ok();
        let payload = sample_payload();
        let req = payload.accepted.clone();
        let mut scheme = MockScheme::escrow();
        scheme.cancel_requirements = Some(req.clone());
        let rs = ResourceServer::new(Arc::clone(&mock)).with_scheme(eip155_wildcard(), scheme);
        let receipt = rs
            .cancellation_guard(payload, req)
            .cancel(CancelReason::HandlerFailed, None, None)
            .await;
        assert!(receipt.is_none());
        assert_eq!(mock.settles.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn cancel_skips_settle_when_settle_on_cancel_returns_none() {
        let mock = mock_ok();
        let payload = sample_payload();
        let req = payload.accepted.clone();
        let rs = ResourceServer::new(Arc::clone(&mock))
            .with_scheme(eip155_wildcard(), MockScheme::escrow());
        let receipt = rs
            .cancellation_guard(payload, req)
            .with_settled_phases([SettlePhase::BeforeHandler])
            .cancel(CancelReason::HandlerFailed, None, None)
            .await;
        assert!(receipt.is_none());
        assert_eq!(mock.settles.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn cancel_settle_error_becomes_failed_receipt() {
        let mock = Arc::new(MockFacilitator {
            verifies: AtomicUsize::new(0),
            settles: AtomicUsize::new(0),
            fail_verify: false,
            fail_settle: true,
        });
        let payload = sample_payload();
        let req = payload.accepted.clone();
        let mut scheme = MockScheme::escrow();
        scheme.cancel_requirements = Some(req.clone());
        let rs = ResourceServer::new(Arc::clone(&mock)).with_scheme(eip155_wildcard(), scheme);
        let receipt = rs
            .cancellation_guard(payload, req)
            .with_settled_phases([SettlePhase::BeforeHandler])
            .cancel(CancelReason::HandlerThrew, None, None)
            .await
            .unwrap();
        assert!(!receipt.is_success());
        assert_eq!(mock.settles.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn cancellation_guard_stores_before_handler_receipt() {
        let payload = sample_payload();
        let req = payload.accepted.clone();
        let settlement = CompletedSettlement::new(
            SettlePhase::BeforeHandler,
            PaymentFlowName::Upfront,
            success_settle("0xdeposit"),
            req.clone(),
        );
        let guard = ResourceServer::new(mock_ok())
            .cancellation_guard(payload, req)
            .with_before_handler(settlement)
            .with_settled_phases([SettlePhase::BeforeHandler]);
        assert_eq!(
            guard.before_handler().unwrap().phase,
            SettlePhase::BeforeHandler
        );
        assert_eq!(guard.settled_phases(), [SettlePhase::BeforeHandler]);
    }
}
