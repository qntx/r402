//! Sequential `handle_request`. Concurrent/Background entries are a later PR.

use std::convert::Infallible;
use std::sync::Arc;

use axum_core::extract::Request;
use axum_core::response::Response;
use r402_protocol::payment::{PriceTag, ResourceInfo, SettleResponse, SettlementOverrides};
use r402_server::{
    AfterHandler, CompletedSettlement, PaymentFlowName, ResourceServer, SequentialFinish,
    SettlePhase, SettlementMode, finish, schedule,
};
use tower::Service;

use super::fail::GateError;
use super::hooks::DynGateHooks;
use super::receipt::{
    attach_failure_path_settlement, attach_payment_response, skip_handler_response,
};
use super::verify::{
    HANDLER_FAILED, VerifiedPayment, cancellation_guard, handler_failed_reason, map_settle,
    payment_flow_of, phases_of, settle_before_handler_if_needed,
};

/// HTTP Sequential payment gate.
pub struct Gate {
    pub(crate) server: ResourceServer,
    pub(crate) accepts: Arc<[PriceTag]>,
    pub(crate) resource: ResourceInfo,
    pub(crate) payment_required: Option<r402_protocol::payment::PaymentRequired>,
    pub(crate) hooks: Option<Arc<dyn DynGateHooks>>,
    pub(crate) settlement_mode: SettlementMode,
}

impl std::fmt::Debug for Gate {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Gate")
            .field("accepts", &self.accepts.len())
            .field("resource", &self.resource)
            .field("settlement_mode", &self.settlement_mode)
            .finish_non_exhaustive()
    }
}

impl Gate {
    pub(crate) fn from_parts(
        server: ResourceServer,
        accepts: Vec<PriceTag>,
        resource: ResourceInfo,
        settlement_mode: SettlementMode,
        hooks: Option<Arc<dyn DynGateHooks>>,
    ) -> Self {
        Self {
            server,
            accepts: accepts.into(),
            resource,
            payment_required: None,
            hooks,
            settlement_mode,
        }
    }

    /// Resource server (facilitator + schemes).
    #[must_use]
    pub const fn resource_server(&self) -> &ResourceServer {
        &self.server
    }

    /// Accepted price tags.
    #[must_use]
    pub fn accepts(&self) -> &[PriceTag] {
        &self.accepts
    }

    /// Resource metadata on 402 bodies.
    #[must_use]
    pub const fn resource(&self) -> &ResourceInfo {
        &self.resource
    }

    /// Built 402 body, if [`Self::build_payment_required`] ran.
    #[must_use]
    pub const fn payment_required(&self) -> Option<&r402_protocol::payment::PaymentRequired> {
        self.payment_required.as_ref()
    }

    /// Requires a registered scheme for every accept. Escrow needs `settle_on_cancel`.
    ///
    /// # Errors
    ///
    /// [`GateError::MissingScheme`], [`GateError::MissingSettleOnCancel`], or flow errors.
    pub async fn require_schemes(&self) -> Result<(), GateError> {
        for tag in self.accepts.iter() {
            let requirements = &tag.requirements;
            if self
                .server
                .registered_scheme(requirements.scheme.as_str(), &requirements.network)
                .is_none()
            {
                return Err(GateError::MissingScheme {
                    scheme: requirements.scheme.clone(),
                    network: requirements.network.clone(),
                });
            }
            let flow = payment_flow_of(&self.server, requirements)?;
            if flow == PaymentFlowName::Escrow
                && !self.server.has_settle_on_cancel(requirements).await
            {
                return Err(GateError::MissingSettleOnCancel {
                    scheme: requirements.scheme.clone(),
                });
            }
        }
        Ok(())
    }

    /// Concurrent/Background are illegal when any resolved accept is upfront or escrow.
    ///
    /// # Errors
    ///
    /// [`GateError::IncompatibleSettlementMode`] or flow-resolution errors.
    pub fn assert_mode_compatible(&self, mode: SettlementMode) -> Result<(), GateError> {
        if mode == SettlementMode::Sequential {
            return Ok(());
        }
        for tag in self.accepts.iter() {
            let flow = payment_flow_of(&self.server, &tag.requirements)?;
            let phases = flow.phases();
            if let Err(err) = schedule(phases, mode) {
                return Err(GateError::IncompatibleSettlementMode {
                    mode: err.mode,
                    flow: err.flow,
                });
            }
        }
        Ok(())
    }

    /// Sequential: verify → settle-before → handler → finish(after-handler).
    ///
    /// Handler 4xx/5xx never enter [`finish`]. Transport maps to 502 at the layer.
    ///
    /// # Errors
    ///
    /// [`GateError`] for verify/settle/header failures.
    pub async fn handle_request<S>(&self, inner: S, mut req: Request) -> Result<Response, GateError>
    where
        S: Service<Request, Response = Response, Error = Infallible>,
        S::Future: Send,
    {
        let verified = self.verify_only(req.headers()).await?;
        if let Some(h) = self.hooks.as_ref() {
            h.on_payment_verified(&mut req).await;
        }
        if let Some(directive) = verified.skip_handler.clone() {
            let receipt = sequential_after_handler(&verified, None, None).await?;
            return skip_handler_response(&directive, &receipt);
        }

        let (flow, phases) = phases_of(&verified)?;
        let before_handler = settle_before_handler_if_needed(&verified, flow, phases).await?;
        let cancel = cancellation_guard(&verified, before_handler.as_ref());

        let response = match call_inner(inner, req).await {
            Ok(r) => r,
            Err(never) => match never {},
        };

        if response.status().is_client_error() || response.status().is_server_error() {
            let cancel_settlement = cancel
                .cancel(
                    handler_failed_reason(),
                    Some(HANDLER_FAILED),
                    Some(response.status().as_u16()),
                )
                .await;
            let mut response = response;
            attach_failure_path_settlement(
                &mut response,
                cancel_settlement.as_ref(),
                before_handler.as_ref(),
                Some(&verified.payload),
            )?;
            return Ok(response);
        }

        let mut response = response;
        let override_amount = super::overrides::resolve_response_settlement_amount(
            &mut response,
            verified.requirements(),
        )
        .map_err(|e| GateError::SettlementAborted(e.to_string()))?;
        let overrides = override_amount.as_deref().map(SettlementOverrides::amount);

        let settlement =
            sequential_after_handler(&verified, overrides.as_ref(), before_handler.as_ref())
                .await?;
        attach_payment_response(&mut response, &settlement)?;
        Ok(response)
    }
}

async fn sequential_after_handler(
    verified: &VerifiedPayment,
    overrides: Option<&SettlementOverrides>,
    before_handler: Option<&CompletedSettlement>,
) -> Result<SettleResponse, GateError> {
    let (_, phases) = phases_of(verified)?;
    let scheduled = schedule(phases, SettlementMode::Sequential).map_err(|err| {
        GateError::IncompatibleSettlementMode {
            mode: err.mode,
            flow: err.flow,
        }
    })?;

    match scheduled.after_handler() {
        AfterHandler::WaitThenSettle => {
            let fut = verified.settle_phase(SettlePhase::AfterHandler, overrides);
            match finish(scheduled, Some(fut)).await {
                Ok(SequentialFinish::Settled(receipt)) => map_settle(Ok(receipt)),
                Ok(SequentialFinish::Echo) => Ok(echo_or_empty(before_handler, verified)),
                Err(err) => Err(GateError::from_settle_facilitator(err)),
            }
        }
        AfterHandler::EchoReceipt => match finish(
            scheduled,
            None::<
                std::future::Ready<Result<SettleResponse, r402_protocol::error::FacilitatorError>>,
            >,
        )
        .await
        {
            Ok(SequentialFinish::Echo) => Ok(echo_or_empty(before_handler, verified)),
            Ok(SequentialFinish::Settled(receipt)) => map_settle(Ok(receipt)),
            Err(err) => Err(GateError::from_settle_facilitator(err)),
        },
        AfterHandler::JoinSettle | AfterHandler::SpawnSettle => {
            Err(GateError::PaymentRequiredBuild(
                "sequential handle_request cannot run concurrent/background finish".into(),
            ))
        }
    }
}

fn echo_or_empty(
    before_handler: Option<&CompletedSettlement>,
    verified: &VerifiedPayment,
) -> SettleResponse {
    if let Some(completed) = before_handler {
        return completed.result.clone();
    }
    SettleResponse::Success {
        payer: compact_str::CompactString::default(),
        transaction: compact_str::CompactString::default(),
        network: verified.requirements().network.to_string().into(),
        amount: None,
        extensions: r402_protocol::payment::Extensions::new(),
        extension_responses: r402_protocol::payment::Extensions::new(),
        extra: None,
    }
}

async fn call_inner<S>(mut inner: S, req: Request) -> Result<Response, Infallible>
where
    S: Service<Request, Response = Response, Error = Infallible>,
    S::Future: Send,
{
    inner.call(req).await
}
