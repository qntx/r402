//! Resource-server orchestration shared by HTTP and MCP transports.
//!
//! Mirrors the official Go/TS `X402ResourceServer` **V2** surface used by
//! transports: match requirements, verify, settle, and lifecycle hooks
//! including verified-payment cancellation.
//!
//! This type does **not** own pricing or route maps. Registered
//! [`SchemeNetworkServer`] adapters resolve payment flow; verify and settle
//! still go through a [`Facilitator`].

mod hook_policy;
mod hooks;
mod payment_flow;
mod scheme;

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
    apply_payment_flow_wire_extra, extra_payment_flow, is_authorization_payment_flow,
    is_recognized_payment_flow, resolve_payment_flow, resolve_payment_flow_phases,
};
pub use scheme::{DynSchemeNetworkServer, SchemeNetworkServer, SchemePaymentRequiredContext};
use serde::Serialize;
use serde_json::Value;

use crate::chain::{ChainId, ChainIdPattern};
use crate::error::{FacilitatorError, VerificationError};
use crate::extensions::{AdvertiseContext, Extension, ExtensionRegistry};
use crate::facilitator::FailureRecovery;
use crate::facilitator::{DynFacilitator, Facilitator};
use crate::wire::{
    Extensions, PaymentRequired, PaymentRequirements, ResourceInfo, SettleRequest, SettleResponse,
    SupportedResponse, TypedVerifyRequest, V2, VerifyRequest, VerifyResponse,
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
    schemes: Vec<(ChainIdPattern, Arc<dyn DynSchemeNetworkServer>)>,
    extensions: ExtensionRegistry,
}

impl Clone for ResourceServer {
    fn clone(&self) -> Self {
        Self {
            facilitator: Arc::clone(&self.facilitator),
            hooks: self.hooks.clone(),
            schemes: self.schemes.clone(),
            extensions: self.extensions.clone(),
        }
    }
}

impl std::fmt::Debug for ResourceServer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ResourceServer")
            .field("hooks", &self.hooks.len())
            .field("schemes", &self.schemes.len())
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
            schemes: Vec::new(),
            extensions: ExtensionRegistry::new(),
        }
    }

    /// Creates from an already-erased facilitator handle.
    #[must_use]
    pub fn from_dyn(facilitator: Arc<dyn DynFacilitator>) -> Self {
        Self {
            facilitator,
            hooks: Vec::new(),
            schemes: Vec::new(),
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

    /// Resolves ATM + payment flow from the registered scheme table.
    ///
    /// # Errors
    ///
    /// [`PaymentFlowError::UnregisteredScheme`] when no adapter matches this
    /// accept, or ATM/flow errors from [`resolve_payment_flow`].
    fn resolved_payment_flow(
        &self,
        requirements: &PaymentRequirements,
    ) -> Result<ResolvedPaymentFlow, PaymentFlowError> {
        let Some(scheme) =
            self.registered_scheme(requirements.scheme.as_str(), &requirements.network)
        else {
            return Err(PaymentFlowError::UnregisteredScheme {
                scheme: requirements.scheme.to_string(),
                network: requirements.network.to_string(),
            });
        };
        resolve_payment_flow(
            &PaymentFlowScheme {
                scheme: scheme.scheme(),
                default_asset_transfer_method: scheme.default_asset_transfer_method(),
                payment_flows: scheme.payment_flows(),
            },
            requirements,
        )
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
        Ok(self.resolved_payment_flow(requirements)?.payment_flow)
    }

    /// Whether the registered scheme yields cancel-settle requirements.
    ///
    /// Escrow accepts must return `true` before HTTP/MCP will serve them.
    pub async fn has_settle_on_cancel(&self, requirements: &PaymentRequirements) -> bool {
        let Some(scheme) =
            self.registered_scheme(requirements.scheme.as_str(), &requirements.network)
        else {
            return false;
        };
        let ctx = VerifiedPaymentCanceledContext {
            payment: PaymentHookContext {
                payload: WirePaymentPayload::new(requirements.clone(), Value::Null),
                requirements: requirements.clone(),
            },
            reason: CancelReason::HandlerFailed,
            error: None,
            response_status: None,
            settled_phases: vec![SettlePhase::BeforeHandler],
        };
        scheme.settle_on_cancel(&ctx).await.is_some()
    }

    /// Registers a protocol extension advertised on 402 responses.
    #[must_use]
    pub fn with_extension(mut self, extension: impl Extension + 'static) -> Self {
        self.extensions.register(extension);
        self
    }

    /// Official `CreatePaymentRequiredResponse`: one 402 body used for both
    /// unpaid challenges and paid matching.
    ///
    /// After scheme enrich and extension advertise, each accept is resolved
    /// through the registered scheme table (official `resolvePaymentFlow`) and
    /// written with official `applyPaymentFlowWireExtra`. Unregistered schemes
    /// fail closed.
    ///
    /// # Errors
    ///
    /// [`FacilitatorError::Internal`] when hook policy, unregistered scheme,
    /// or payment-flow resolution / wire extra application fails.
    pub async fn create_payment_required_response(
        &self,
        accepts: Vec<PaymentRequirements>,
        ctx: PaymentRequiredBuildContext,
    ) -> Result<PaymentRequired, FacilitatorError> {
        let PaymentRequiredBuildContext {
            resource,
            error,
            extensions,
            supported,
            payment_payload,
        } = ctx;
        let mut response = PaymentRequired::new(resource)
            .with_accepts(accepts)
            .with_extensions(extensions);
        if let Some(error) = error {
            response = response.with_error(error);
        }
        self.apply_scheme_payment_required_enrich(
            &mut response,
            &supported,
            payment_payload.as_ref(),
        )
        .await?;
        advertise_registered_extensions(&self.extensions, &mut response)?;
        self.apply_payment_flow_extras(&mut response.accepts)?;
        Ok(response)
    }

    /// Official `BuildPaymentRequirements` wire step: scheme-table resolve, then
    /// [`apply_payment_flow_wire_extra`].
    ///
    /// Omit ATM → scheme default. Omit `paymentFlow` → that ATM's table default.
    /// Non-authorization flows are written onto `extra.paymentFlow`. The SDK
    /// ATM sentinel `"default"` is stripped. Unregistered schemes fail closed.
    fn apply_payment_flow_extras(
        &self,
        accepts: &mut [PaymentRequirements],
    ) -> Result<(), FacilitatorError> {
        for accept in accepts {
            let resolved = self
                .resolved_payment_flow(accept)
                .map_err(FacilitatorError::internal)?;
            let next = apply_payment_flow_wire_extra(accept.extra.as_ref(), &resolved);
            accept.extra = if next.is_empty() {
                None
            } else {
                Some(Value::Object(next))
            };
        }
        Ok(())
    }

    /// Official per-accept scheme enrich, then additive-extra hook policy.
    async fn apply_scheme_payment_required_enrich(
        &self,
        response: &mut PaymentRequired,
        supported: &SupportedResponse,
        payment_payload: Option<&WirePaymentPayload>,
    ) -> Result<(), FacilitatorError> {
        let targets: Vec<(CompactString, ChainId)> = response
            .accepts
            .iter()
            .map(|accept| (accept.scheme.clone(), accept.network.clone()))
            .collect();
        let mut baseline = snapshot_payment_requirements_list(&response.accepts);
        for (scheme_name, network) in targets {
            let Some(scheme) = self.registered_scheme(scheme_name.as_str(), &network) else {
                continue;
            };
            let network_label = network.to_string();
            let enriched = {
                let ctx = SchemePaymentRequiredContext {
                    requirements: &response.accepts,
                    payment_payload,
                    resource: &response.resource,
                    error: response.error.as_deref(),
                    payment_required_response: response,
                    supported,
                };
                scheme.enrich_payment_required_response(&ctx).await
            };
            if let Some(accepts) = enriched {
                response.accepts = accepts;
            }
            assert_accepts_additive_extra_after_scheme_enrich(
                &baseline,
                &response.accepts,
                scheme_name.as_str(),
                network_label.as_str(),
            )
            .map_err(FacilitatorError::internal)?;
            baseline = snapshot_payment_requirements_list(&response.accepts);
        }
        Ok(())
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
    /// Every accept must have a registered [`SchemeNetworkServer`]. Empty
    /// `schemes` is a programming error ([`PaymentFlowError::UnregisteredScheme`]),
    /// not an always-facilitator fallback. When the resolved flow has
    /// `!verify_before_handler`, returns Valid without the facilitator and
    /// without `after_verify`.
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
        let payload = self
            .apply_scheme_settlement_payload_enrich(payload, &effective, phase)
            .await?;
        let settle_ctx = SettleContext {
            payment: PaymentHookContext {
                payload,
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

    /// Official pre-settle payload enrich (SVM `upto` voucher on claim/cancel).
    async fn apply_scheme_settlement_payload_enrich(
        &self,
        payload: &WirePaymentPayload,
        requirements: &PaymentRequirements,
        phase: SettlePhase,
    ) -> Result<WirePaymentPayload, FacilitatorError> {
        let Some(scheme) =
            self.registered_scheme(requirements.scheme.as_str(), &requirements.network)
        else {
            return Ok(payload.clone());
        };
        let ctx = SettleContext {
            payment: PaymentHookContext {
                payload: payload.clone(),
                requirements: requirements.clone(),
            },
            declared_extensions: Extensions::new(),
            phase,
        };
        let Some(enrichment) = scheme.enrich_settlement_payload(&ctx).await? else {
            return Ok(payload.clone());
        };
        let mut enriched = payload.clone();
        let Value::Object(payload_map) = &mut enriched.payload else {
            return Err(FacilitatorError::Verification(
                VerificationError::InvalidFormat("settlement payload is not a JSON object".into()),
            ));
        };
        assert_additive_payload_enrichment(payload_map, &enrichment, scheme.scheme())
            .map_err(FacilitatorError::internal)?;
        for (key, value) in enrichment {
            payload_map.insert(key, value);
        }
        Ok(enriched)
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

/// Settlement receipt attached when the resource handler fails after verify
/// (Go `BuildFailurePathSettlementResponse`).
///
/// Preference: successful cancel settle, then failed cancel with deposit-recovery
/// extras, then before-handler echo, else `None`.
#[must_use]
pub fn build_failure_path_settlement_response(
    cancel_settlement: Option<&SettleResponse>,
    before_handler: Option<&CompletedSettlement>,
    payment_payload: Option<&WirePaymentPayload>,
) -> Option<SettleResponse> {
    if let Some(cancel) = cancel_settlement {
        if cancel.is_success() {
            return Some(cancel.clone());
        }
        return Some(build_failed_cancel_receipt(
            cancel,
            before_handler,
            payment_payload,
        ));
    }
    before_handler.map(|completed| completed.result.clone())
}

/// Failed cancel receipt with deposit-recovery facts in `extra`.
fn build_failed_cancel_receipt(
    cancel: &SettleResponse,
    before_handler: Option<&CompletedSettlement>,
    payment_payload: Option<&WirePaymentPayload>,
) -> SettleResponse {
    let SettleResponse::Failure {
        reason,
        message,
        payer,
        network,
        extensions,
        extra: cancel_extra,
        ..
    } = cancel
    else {
        return cancel.clone();
    };

    let mut extra_map = match cancel_extra {
        Some(Value::Object(map)) => map.clone(),
        _ => serde_json::Map::new(),
    };
    if let Some(before) = before_handler {
        extra_map.insert(
            "depositTransaction".into(),
            Value::String(settle_response_transaction(&before.result).to_owned()),
        );
        extra_map.insert(
            "depositAmount".into(),
            Value::String(
                settle_response_amount(&before.result)
                    .unwrap_or("")
                    .to_owned(),
            ),
        );
    }
    if let Some(payload) = payment_payload
        && let Some(channel_id) = payload
            .payload
            .get("channelId")
            .and_then(Value::as_str)
            .filter(|id| !id.is_empty())
    {
        extra_map.insert("channelId".into(), Value::String(channel_id.to_owned()));
    }
    let merged_extra = if extra_map.is_empty() {
        None
    } else {
        Some(Value::Object(extra_map))
    };
    SettleResponse::Failure {
        reason: reason.clone(),
        message: message.clone(),
        payer: payer.clone(),
        transaction: CompactString::default(),
        network: network.clone(),
        extensions: extensions.clone(),
        extra: merged_extra,
    }
}

fn settle_response_transaction(response: &SettleResponse) -> &str {
    match response {
        SettleResponse::Success { transaction, .. }
        | SettleResponse::Failure { transaction, .. } => transaction.as_str(),
    }
}

fn settle_response_amount(response: &SettleResponse) -> Option<&str> {
    match response {
        SettleResponse::Success { amount, .. } => amount.as_deref(),
        SettleResponse::Failure { .. } => None,
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
    use crate::wire::{Extensions, ResourceInfo, SettleRequest, SupportedResponse, VerifyRequest};

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
        let rs = ResourceServer::new(Arc::clone(&mock))
            .with_scheme(eip155_wildcard(), MockScheme::authorization());
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
        let rs = ResourceServer::new(mock_ok())
            .with_scheme(eip155_wildcard(), MockScheme::authorization())
            .with_hook(AbortBeforeVerify);
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
        let rs = ResourceServer::new(Arc::clone(&mock))
            .with_scheme(eip155_wildcard(), MockScheme::authorization())
            .with_hook(SkipVerify);
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
            .with_scheme(eip155_wildcard(), MockScheme::authorization())
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
        let rs = ResourceServer::new(mock_ok())
            .with_scheme(eip155_wildcard(), MockScheme::authorization())
            .with_hook(SkipHandlerHook);
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
        let rs = ResourceServer::new(mock)
            .with_scheme(eip155_wildcard(), MockScheme::authorization())
            .with_hook(RecoverVerify);
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
        let rs = ResourceServer::new(mock_ok())
            .with_scheme(eip155_wildcard(), MockScheme::authorization())
            .with_hook(CancelCountHook(Arc::clone(&count)));
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

    struct VoucherEnrichScheme {
        flows: HashMap<String, PaymentFlowConfig>,
    }

    impl SchemeNetworkServer for VoucherEnrichScheme {
        fn scheme(&self) -> &'static str {
            "exact"
        }

        fn default_asset_transfer_method(&self) -> &str {
            SDK_DEFAULT_ASSET_TRANSFER_METHOD
        }

        fn payment_flows(&self) -> &HashMap<String, PaymentFlowConfig> {
            &self.flows
        }

        fn enrich_settlement_payload<'a>(
            &'a self,
            ctx: &'a SettleContext,
        ) -> impl Future<
            Output = Result<Option<serde_json::Map<String, Value>>, FacilitatorError>,
        > + Send
        + 'a {
            let enrichment = match ctx.phase {
                SettlePhase::BeforeHandler => None,
                SettlePhase::AfterHandler | SettlePhase::Cancel => {
                    let mut map = serde_json::Map::new();
                    map.insert("voucherSignature".into(), Value::String("sig".into()));
                    Some(map)
                }
            };
            std::future::ready(Ok(enrichment))
        }
    }

    struct CaptureSettlePayload {
        payloads: Mutex<Vec<Value>>,
    }

    impl Facilitator for CaptureSettlePayload {
        fn verify(
            &self,
            _: VerifyRequest,
        ) -> impl Future<Output = Result<VerifyResponse, FacilitatorError>> + Send {
            std::future::ready(Ok(VerifyResponse::valid("0x")))
        }

        fn settle(
            &self,
            request: SettleRequest,
        ) -> impl Future<Output = Result<SettleResponse, FacilitatorError>> + Send {
            let json = request.into_json();
            self.payloads
                .lock()
                .unwrap()
                .push(json["paymentPayload"]["payload"].clone());
            std::future::ready(Ok(success_settle("0x1")))
        }

        fn supported(
            &self,
        ) -> impl Future<Output = Result<SupportedResponse, FacilitatorError>> + Send {
            std::future::ready(Ok(SupportedResponse::default()))
        }
    }

    #[tokio::test]
    async fn settle_payment_merges_scheme_payload_enrichment_after_handler() {
        let mut flows = HashMap::new();
        flows.insert(
            SDK_DEFAULT_ASSET_TRANSFER_METHOD.into(),
            PaymentFlowConfig::authorization_and_upfront(),
        );
        let fac = Arc::new(CaptureSettlePayload {
            payloads: Mutex::new(Vec::new()),
        });
        let rs = ResourceServer::new(Arc::clone(&fac))
            .with_scheme(eip155_wildcard(), VoucherEnrichScheme { flows });
        let payload = sample_payload();
        let req = payload.accepted.clone();

        rs.settle_payment(&payload, &req, None, SettlePhase::BeforeHandler)
            .await
            .unwrap();
        rs.settle_payment(&payload, &req, None, SettlePhase::AfterHandler)
            .await
            .unwrap();
        rs.settle_payment(&payload, &req, None, SettlePhase::Cancel)
            .await
            .unwrap();

        let captured = fac.payloads.lock().unwrap().clone();
        assert_eq!(captured[0], serde_json::json!({"k": 1}));
        assert_eq!(
            captured[1],
            serde_json::json!({"k": 1, "voucherSignature": "sig"})
        );
        assert_eq!(
            captured[2],
            serde_json::json!({"k": 1, "voucherSignature": "sig"})
        );
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

        /// SVM `upto` table: ATM `channel`, escrow-only, default escrow.
        fn upto_channel_escrow() -> Self {
            let mut scheme = Self::escrow();
            scheme.name = "upto";
            scheme.default_atm = "channel";
            scheme.flows = HashMap::from([(
                "channel".into(),
                PaymentFlowConfig::new(vec![PaymentFlowName::Escrow], PaymentFlowName::Escrow),
            )]);
            scheme
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

    fn sample_build_ctx() -> PaymentRequiredBuildContext {
        PaymentRequiredBuildContext {
            resource: ResourceInfo::new("https://example.com/paid"),
            error: None,
            extensions: Extensions::new(),
            supported: SupportedResponse::default(),
            payment_payload: None,
        }
    }

    #[tokio::test]
    async fn verify_payment_errors_when_table_empty() {
        let mock = mock_ok();
        let rs = ResourceServer::new(Arc::clone(&mock));
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

    struct FeePayerEnrichScheme {
        flows: HashMap<String, PaymentFlowConfig>,
    }

    impl SchemeNetworkServer for FeePayerEnrichScheme {
        fn scheme(&self) -> &'static str {
            "exact"
        }

        fn default_asset_transfer_method(&self) -> &str {
            SDK_DEFAULT_ASSET_TRANSFER_METHOD
        }

        fn payment_flows(&self) -> &HashMap<String, PaymentFlowConfig> {
            &self.flows
        }

        fn enrich_payment_required_response<'a>(
            &'a self,
            ctx: &'a SchemePaymentRequiredContext<'a>,
        ) -> impl Future<Output = Option<Vec<PaymentRequirements>>> + Send + 'a {
            let fee_payer = ctx.supported.kinds.iter().find_map(|kind| {
                kind.extra
                    .as_ref()
                    .and_then(|extra| extra.get("feePayer"))
                    .cloned()
            });
            let mut accepts = ctx.requirements.to_vec();
            let Some(fee_payer) = fee_payer else {
                return std::future::ready(None);
            };
            for req in &mut accepts {
                let mut extra = req
                    .extra
                    .take()
                    .and_then(|value| value.as_object().cloned())
                    .unwrap_or_default();
                extra.insert("feePayer".into(), fee_payer.clone());
                req.extra = Some(Value::Object(extra));
            }
            std::future::ready(Some(accepts))
        }
    }

    #[tokio::test]
    async fn create_payment_required_runs_scheme_enrich() {
        let supported = SupportedResponse::new().with_kinds(vec![
            crate::wire::SupportedPaymentKind::new(2, "exact", "eip155:1")
                .with_extra(serde_json::json!({ "feePayer": "0xfee" })),
        ]);
        let mut flows = HashMap::new();
        flows.insert(
            SDK_DEFAULT_ASSET_TRANSFER_METHOD.into(),
            PaymentFlowConfig::authorization_and_upfront(),
        );
        let rs = ResourceServer::new(mock_ok())
            .with_scheme(eip155_wildcard(), FeePayerEnrichScheme { flows });
        let req = sample_payload().accepted;
        let built = rs
            .create_payment_required_response(
                vec![req],
                PaymentRequiredBuildContext {
                    resource: ResourceInfo::new("https://example.com/paid"),
                    error: None,
                    extensions: Extensions::new(),
                    supported,
                    payment_payload: None,
                },
            )
            .await
            .unwrap();
        let extra = built
            .accepts
            .first()
            .and_then(|accept| accept.extra.as_ref());
        assert_eq!(
            extra.and_then(|value| value.get("feePayer")),
            Some(&Value::String("0xfee".into()))
        );
        assert!(extra.and_then(|value| value.get("paymentFlow")).is_none());
        assert!(
            extra
                .and_then(|value| value.get("assetTransferMethod"))
                .is_none()
        );
    }

    #[tokio::test]
    async fn create_payment_required_omits_authorization_payment_flow() {
        let rs = ResourceServer::new(mock_ok())
            .with_scheme(eip155_wildcard(), MockScheme::authorization());
        let built = rs
            .create_payment_required_response(vec![sample_payload().accepted], sample_build_ctx())
            .await
            .unwrap();
        let extra = built
            .accepts
            .first()
            .and_then(|accept| accept.extra.as_ref());
        assert!(extra.and_then(|value| value.get("paymentFlow")).is_none());
        assert!(
            extra
                .and_then(|value| value.get("assetTransferMethod"))
                .is_none()
        );
        assert!(is_authorization_payment_flow(extra));
    }

    #[tokio::test]
    async fn create_payment_required_writes_scheme_default_escrow_payment_flow() {
        let rs = ResourceServer::new(mock_ok()).with_scheme(
            ChainIdPattern::wildcard("solana"),
            MockScheme::upto_channel_escrow(),
        );
        let req = PaymentRequirements::new(
            "upto".into(),
            "solana:mainnet".parse().unwrap(),
            "1".into(),
            "PayTo".into(),
            "Mint".into(),
            60,
        );
        assert_eq!(rs.get_payment_flow(&req).unwrap(), PaymentFlowName::Escrow);
        let built = rs
            .create_payment_required_response(vec![req], sample_build_ctx())
            .await
            .unwrap();
        let extra = built
            .accepts
            .first()
            .and_then(|accept| accept.extra.as_ref());
        assert_eq!(
            extra
                .and_then(|value| value.get("paymentFlow"))
                .and_then(Value::as_str),
            Some("escrow")
        );
        assert!(
            extra
                .and_then(|value| value.get("assetTransferMethod"))
                .is_none()
        );
        assert!(!is_authorization_payment_flow(extra));
    }

    #[tokio::test]
    async fn create_payment_required_writes_upfront_when_scheme_defaults_to_upfront() {
        let rs =
            ResourceServer::new(mock_ok()).with_scheme(eip155_wildcard(), MockScheme::upfront());
        let built = rs
            .create_payment_required_response(vec![sample_payload().accepted], sample_build_ctx())
            .await
            .unwrap();
        assert_eq!(
            built
                .accepts
                .first()
                .and_then(|accept| accept.extra.as_ref())
                .and_then(|extra| extra.get("paymentFlow"))
                .and_then(Value::as_str),
            Some("upfront")
        );
    }

    #[tokio::test]
    async fn create_payment_required_strips_sdk_atm_sentinel() {
        let rs = ResourceServer::new(mock_ok())
            .with_scheme(eip155_wildcard(), MockScheme::authorization());
        let mut req = sample_payload().accepted;
        req.extra = Some(serde_json::json!({
            "assetTransferMethod": SDK_DEFAULT_ASSET_TRANSFER_METHOD,
            "name": "USDC",
        }));
        let built = rs
            .create_payment_required_response(vec![req], sample_build_ctx())
            .await
            .unwrap();
        let extra = built
            .accepts
            .first()
            .and_then(|accept| accept.extra.as_ref())
            .expect("extra retained");
        assert_eq!(extra.get("name").and_then(Value::as_str), Some("USDC"));
        assert!(extra.get("assetTransferMethod").is_none());
        assert!(extra.get("paymentFlow").is_none());
    }

    #[tokio::test]
    async fn create_payment_required_fails_unregistered_scheme() {
        let rs = ResourceServer::new(mock_ok());
        let err = rs
            .create_payment_required_response(vec![sample_payload().accepted], sample_build_ctx())
            .await
            .unwrap_err();
        assert!(
            matches!(err, FacilitatorError::Internal(_))
                && err.to_string().contains(
                    "No server implementation registered for scheme: exact, network: eip155:1"
                ),
            "{err}"
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
    async fn has_settle_on_cancel_probes_scheme() {
        let req = sample_payload().accepted;
        let none =
            ResourceServer::new(mock_ok()).with_scheme(eip155_wildcard(), MockScheme::escrow());
        assert!(!none.has_settle_on_cancel(&req).await);

        let mut with_cancel = MockScheme::escrow();
        with_cancel.cancel_requirements = Some(req.clone());
        let some = ResourceServer::new(mock_ok()).with_scheme(eip155_wildcard(), with_cancel);
        assert!(some.has_settle_on_cancel(&req).await);
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

    #[test]
    fn failure_path_prefers_successful_cancel() {
        let payload = sample_payload();
        let req = payload.accepted;
        let before = CompletedSettlement::new(
            SettlePhase::BeforeHandler,
            PaymentFlowName::Escrow,
            success_settle("0xdeposit"),
            req,
        );
        let cancel = success_settle("0xrefund");
        let got = build_failure_path_settlement_response(Some(&cancel), Some(&before), None);
        assert!(matches!(
            got,
            Some(SettleResponse::Success { ref transaction, .. }) if transaction == "0xrefund"
        ));
    }

    #[test]
    fn failure_path_failed_cancel_includes_deposit_recovery() {
        let payload = WirePaymentPayload::new(
            sample_payload().accepted,
            serde_json::json!({"channelId": "channel-123"}),
        );
        let req = payload.accepted.clone();
        let before = CompletedSettlement::new(
            SettlePhase::BeforeHandler,
            PaymentFlowName::Escrow,
            SettleResponse::Success {
                payer: "0xpayer".into(),
                transaction: "0xdeposit".into(),
                network: "eip155:8453".into(),
                amount: Some("1000".into()),
                extensions: Extensions::new(),
                extra: None,
            },
            req,
        );
        let cancel = SettleResponse::Failure {
            reason: crate::error::ErrorReason::UnexpectedSettleError,
            message: Some("refund_failed".into()),
            payer: None,
            transaction: "should-clear".into(),
            network: "eip155:8453".into(),
            extensions: Extensions::new(),
            extra: None,
        };
        let got =
            build_failure_path_settlement_response(Some(&cancel), Some(&before), Some(&payload))
                .unwrap();
        match got {
            SettleResponse::Failure {
                ref transaction,
                ref extra,
                ..
            } => {
                assert!(transaction.is_empty());
                let extra = extra.as_ref().and_then(Value::as_object).unwrap();
                assert_eq!(
                    extra.get("depositTransaction"),
                    Some(&Value::from("0xdeposit"))
                );
                assert_eq!(extra.get("depositAmount"), Some(&Value::from("1000")));
                assert_eq!(extra.get("channelId"), Some(&Value::from("channel-123")));
            }
            other => panic!("expected failure receipt, got {other:?}"),
        }
    }

    #[test]
    fn failure_path_echoes_before_handler_when_cancel_absent() {
        let req = sample_payload().accepted;
        let before = CompletedSettlement::new(
            SettlePhase::BeforeHandler,
            PaymentFlowName::Upfront,
            success_settle("0xdeposit"),
            req,
        );
        let got = build_failure_path_settlement_response(None, Some(&before), None);
        assert!(matches!(
            got,
            Some(SettleResponse::Success { ref transaction, .. }) if transaction == "0xdeposit"
        ));
    }

    #[test]
    fn failure_path_none_when_nothing_available() {
        assert!(build_failure_path_settlement_response(None, None, None).is_none());
    }
}
