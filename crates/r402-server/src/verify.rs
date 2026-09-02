//! Verify-payment orchestration.

use r402_facilitator::{DynFacilitator, FailureRecovery};
use r402_protocol::error::{FacilitatorError, VerificationError};
use r402_protocol::payment::{
    PaymentRequirements, TypedVerifyRequest, V2, VerifyRequest, VerifyResponse,
};
use serde::Serialize;

use crate::hooks::{
    AfterVerifyDecision, BeforeOpDecision, CancelReason, PaymentHookContext, SkipHandlerDirective,
    VerifyResultContext, WirePaymentPayload,
};
use crate::payment_flow::resolve_payment_flow_phases;
use crate::resource::ResourceServer;

/// Successful verify path outcome (after hooks).
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct VerifyPaymentOutcome {
    /// Facilitator (or skipped/recovered) verify response.
    pub response: VerifyResponse,
    /// When set, the transport must skip the resource handler and settle inline.
    pub skip_handler: Option<SkipHandlerDirective>,
}

impl ResourceServer {
    /// Before hooks → facilitator → after hooks.
    ///
    /// Every accept must have a registered [`crate::SchemeNetworkServer`]. Empty
    /// `schemes` is [`crate::PaymentFlowError::UnregisteredScheme`], not an
    /// always-facilitator fallback. When the resolved flow has
    /// `!verify_before_handler`, returns Valid without the facilitator and
    /// without `after_verify`.
    ///
    /// # Errors
    ///
    /// Propagates facilitator errors (unless recovered), before-hook aborts,
    /// after-verify aborts ([`FacilitatorError::Aborted`]), and payment-flow
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
        record_verify_metrics(&result, started.elapsed());
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
}

pub(crate) fn build_verify_request(
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

pub(crate) fn to_verify_request<T: Serialize>(
    typed: &T,
) -> Result<VerifyRequest, FacilitatorError> {
    let json = serde_json::to_value(typed).map_err(FacilitatorError::internal)?;
    Ok(VerifyRequest::from(json))
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
        _ => FacilitatorError::Verification(VerificationError::InvalidFormat(
            "expected invalid verify response".into(),
        )),
    }
}

#[cfg(feature = "metrics")]
fn record_verify_metrics(
    result: &Result<VerifyResponse, FacilitatorError>,
    elapsed: std::time::Duration,
) {
    let label = match result {
        Ok(r) if r.is_valid() => "valid",
        Ok(_) => "invalid",
        Err(_) => "error",
    };
    ::metrics::counter!(
        r402_protocol::metrics::FACILITATOR_VERIFY_TOTAL,
        "result" => label,
    )
    .increment(1);
    ::metrics::histogram!(r402_protocol::metrics::FACILITATOR_VERIFY_DURATION_SECONDS)
        .record(elapsed.as_secs_f64());
}
