//! Steps 3–5: extract `Payment-Signature`, match, verify, skip-handler, settle-before.

use http::HeaderMap;
use r402_facilitator::DynFacilitator;
use r402_protocol::error::{FacilitatorError, FacilitatorTransportKind};
use r402_protocol::payment::{
    Base64Bytes, Extensions, PaymentRequired, PaymentRequirements, SettleResponse,
    SettlementOverrides, VerifyResponse,
};
use r402_server::{
    CancelReason, CompletedSettlement, PaymentFlowName, PaymentFlowPhases,
    PaymentRequiredBuildContext, ResourceServer, SettlePhase, SkipHandlerDirective,
    WirePaymentPayload, resolve_payment_flow_phases,
};

use super::fail::GateError;
use super::gate::Gate;
use crate::headers::PAYMENT_SIGNATURE;

/// Verified payment ready for settle. Consumed by after-handler settle.
#[derive(Debug, Clone)]
pub struct VerifiedPayment {
    pub(crate) payload: WirePaymentPayload,
    pub(crate) requirements: PaymentRequirements,
    pub(crate) server: ResourceServer,
    pub(crate) skip_handler: Option<SkipHandlerDirective>,
    pub(crate) resource_url: compact_str::CompactString,
    pub(crate) advertised: Extensions,
}

impl VerifiedPayment {
    /// One-shot cancel dispatcher.
    #[must_use]
    pub fn cancellation_guard(&self) -> r402_server::CancellationGuard {
        self.server
            .cancellation_guard(self.payload.clone(), self.requirements.clone())
    }

    /// Matched requirements (override resolution).
    #[must_use]
    pub const fn requirements(&self) -> &PaymentRequirements {
        &self.requirements
    }

    pub(crate) async fn settle_phase(
        &self,
        phase: SettlePhase,
        overrides: Option<&SettlementOverrides>,
    ) -> Result<SettleResponse, FacilitatorError> {
        self.server
            .settle_payment(
                &self.payload,
                &self.requirements,
                overrides,
                phase,
                Some(self.resource_url.as_str()),
                Some(&self.advertised),
            )
            .await
    }
}

impl Gate {
    /// Fetches `/supported` and builds the shared 402 body.
    ///
    /// # Errors
    ///
    /// [`GateError::Transport`] when `/supported` fails or returns no kinds.
    /// [`GateError::PaymentRequiredBuild`] when 402 construction fails.
    pub async fn build_payment_required(&mut self) -> Result<(), GateError> {
        let facilitator = self.server.facilitator();
        let supported = DynFacilitator::supported(facilitator.as_ref())
            .await
            .map_err(GateError::from_verify_facilitator)?;
        if supported.kinds.is_empty() {
            return Err(GateError::from_verify_facilitator(
                FacilitatorError::transport(FacilitatorTransportKind::MalformedSuccessBody),
            ));
        }
        let reqs: Vec<_> = self
            .accepts
            .iter()
            .map(|pt| pt.requirements.clone())
            .collect();
        let built = self
            .server
            .create_payment_required_response(
                reqs,
                PaymentRequiredBuildContext {
                    resource: self.resource.clone(),
                    error: None,
                    extensions: Extensions::new(),
                    supported,
                    payment_payload: None,
                },
            )
            .await
            .map_err(|err| GateError::PaymentRequiredBuild(err.to_string()))?;
        self.payment_required = Some(built);
        Ok(())
    }

    /// Verifies `Payment-Signature` against the built 402 accepts.
    ///
    /// # Errors
    ///
    /// Missing/malformed header → 402. Transport → 502. `isValid: false` → 402/412.
    pub async fn verify_only(&self, headers: &HeaderMap) -> Result<VerifiedPayment, GateError> {
        let header_bytes = headers
            .get(PAYMENT_SIGNATURE)
            .map(http::HeaderValue::as_bytes)
            .ok_or(GateError::PaymentHeaderMissing)?;

        let payload =
            decode_payment_payload(header_bytes).ok_or(GateError::InvalidPaymentHeader)?;

        let payment_required: &PaymentRequired =
            self.payment_required.as_ref().ok_or_else(|| {
                GateError::PaymentRequiredBuild(
                    "payment-required response has not been built".into(),
                )
            })?;
        let requirements = self
            .server
            .find_matching_requirements(&payment_required.accepts, &payload)
            .cloned()
            .ok_or(GateError::NoPaymentMatching)?;
        let outcome = self
            .server
            .verify_payment(&payload, &requirements, Some(&payment_required.extensions))
            .await
            .map_err(GateError::from_verify_facilitator)?;

        if let VerifyResponse::Invalid {
            reason, message, ..
        } = &outcome.response
        {
            return Err(GateError::from_invalid_verify(
                reason.clone(),
                message.as_deref(),
            ));
        }

        Ok(VerifiedPayment {
            payload,
            requirements,
            server: self.server.clone(),
            skip_handler: outcome.skip_handler,
            resource_url: payment_required.resource.url.clone(),
            advertised: payment_required.extensions.clone(),
        })
    }
}

pub(crate) fn decode_payment_payload(header_bytes: &[u8]) -> Option<WirePaymentPayload> {
    let decoded = Base64Bytes::from(header_bytes).decode().ok()?;
    serde_json::from_slice(decoded.as_ref()).ok()
}

pub(crate) fn payment_flow_of(
    server: &ResourceServer,
    requirements: &PaymentRequirements,
) -> Result<PaymentFlowName, GateError> {
    server
        .get_payment_flow(requirements)
        .map_err(|err| match err {
            r402_server::PaymentFlowError::UnregisteredScheme { scheme, .. } => {
                GateError::MissingScheme {
                    scheme: scheme.into(),
                    network: requirements.network.clone(),
                }
            }
            other => GateError::PaymentRequiredBuild(other.to_string()),
        })
}

pub(crate) fn cancellation_guard(
    verified: &VerifiedPayment,
    before_handler: Option<&CompletedSettlement>,
) -> r402_server::CancellationGuard {
    let guard = verified.cancellation_guard();
    match before_handler {
        Some(completed) => guard
            .with_settled_phases([SettlePhase::BeforeHandler])
            .with_before_handler(completed.clone()),
        None => guard,
    }
}

pub(crate) async fn settle_before_handler_if_needed(
    verified: &VerifiedPayment,
    flow: PaymentFlowName,
    phases: PaymentFlowPhases,
) -> Result<Option<CompletedSettlement>, GateError> {
    if !phases.settle_before_handler {
        return Ok(None);
    }
    let result = map_settle(
        verified
            .settle_phase(SettlePhase::BeforeHandler, None)
            .await,
    )?;
    Ok(Some(CompletedSettlement::new(
        SettlePhase::BeforeHandler,
        flow,
        result,
        verified.requirements().clone(),
    )))
}

pub(crate) fn map_settle(
    result: Result<SettleResponse, FacilitatorError>,
) -> Result<SettleResponse, GateError> {
    let settlement = result.map_err(GateError::from_settle_facilitator)?;
    if matches!(settlement, SettleResponse::Failure { .. }) {
        return Err(GateError::Settlement(Box::new(settlement)));
    }
    Ok(settlement)
}

pub(crate) fn phases_of(
    verified: &VerifiedPayment,
) -> Result<(PaymentFlowName, PaymentFlowPhases), GateError> {
    let flow = payment_flow_of(&verified.server, verified.requirements())?;
    Ok((flow, resolve_payment_flow_phases(flow)))
}

/// Reason string for handler-failed cancel.
pub(crate) const HANDLER_FAILED: &str = "handler returned error status";

pub(crate) const fn handler_failed_reason() -> CancelReason {
    CancelReason::HandlerFailed
}
