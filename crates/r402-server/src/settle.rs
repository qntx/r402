//! Settle-payment orchestration.

use r402_facilitator::{DynFacilitator, FailureRecovery};
use r402_protocol::error::{FacilitatorError, VerificationError};
use r402_protocol::extension::{ExtensionRegistry, SettleContext as ExtensionSettleContext};
use r402_protocol::payment::{
    Extensions, PaymentPayload, PaymentRequirements, SettleRequest, SettleResponse,
    SettlementOverrides, TypedVerifyRequest, V2, asset_decimals_from_extra,
    resolve_settlement_override_amount,
};
use serde_json::Value;

use crate::hooks::{
    BeforeOpDecision, PaymentHookContext, SettleContext, SettleResultContext, WirePaymentPayload,
    assert_additive_payload_enrichment,
};
use crate::payment_flow::{PaymentFlowName, SettlePhase};
use crate::resource::ResourceServer;
use crate::verify::to_verify_request;

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

impl ResourceServer {
    /// Optional amount overrides → before hooks → facilitator → after hooks.
    ///
    /// Facilitator settle is attempted twice only when the first `Ok` result is
    /// `settlement_pending` with a non-empty `transaction`; no sleep.
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

        let mut response = if let Some(local) = skipped {
            local
        } else {
            self.call_settle_with_failure_hooks(&settle_ctx).await?
        };
        attach_settle_extensions(&self.extensions, &settle_ctx, &mut response).await?;

        let result_ctx = SettleResultContext {
            settle: settle_ctx,
            result: response.clone(),
        };
        for hook in &self.hooks {
            hook.after_settle(&result_ctx).await;
        }

        Ok(response)
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
        record_settle_metrics(&result, started.elapsed());
        result
    }

    /// Calls facilitator settle once, then retries identically iff the first
    /// result is `Ok(SettleResponse::Failure)` with `settlement_pending` and a
    /// non-empty transaction hash. `Err` is never retried.
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

async fn attach_settle_extensions(
    registry: &ExtensionRegistry,
    settle: &SettleContext,
    response: &mut SettleResponse,
) -> Result<(), FacilitatorError> {
    if registry.is_empty() {
        return Ok(());
    }
    let json_payload = json_payment_payload(&settle.payment.payload)?;
    let extra = {
        let ctx =
            ExtensionSettleContext::new(&json_payload, &settle.payment.requirements, response);
        registry.collect_settle(&ctx).await
    };
    response.extensions_mut().extend(extra);
    Ok(())
}

fn json_payment_payload(
    payload: &WirePaymentPayload,
) -> Result<PaymentPayload<Value, Value>, FacilitatorError> {
    let accepted = serde_json::to_value(&payload.accepted).map_err(FacilitatorError::internal)?;
    Ok(PaymentPayload::new(accepted, payload.payload.clone())
        .with_optional_resource(payload.resource.clone())
        .with_extensions(payload.extensions.clone()))
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

/// Applies settlement overrides onto a clone of `requirements`.
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

#[cfg(feature = "metrics")]
fn record_settle_metrics(
    result: &Result<SettleResponse, FacilitatorError>,
    elapsed: std::time::Duration,
) {
    let label = match result {
        Ok(r) if r.is_success() => "success",
        Ok(_) => "failure",
        Err(_) => "error",
    };
    ::metrics::counter!(
        r402_protocol::metrics::FACILITATOR_SETTLE_TOTAL,
        "result" => label,
    )
    .increment(1);
    ::metrics::histogram!(r402_protocol::metrics::FACILITATOR_SETTLE_DURATION_SECONDS)
        .record(elapsed.as_secs_f64());
}
