//! Three Gate entries. Verify (steps 1–5) lives in [`super::verify`].

use std::convert::Infallible;
use std::future::Future;
#[cfg(feature = "siwx")]
use std::pin::Pin;
use std::sync::Arc;

use axum_core::extract::Request;
use axum_core::response::Response;
use r402_protocol::error::FacilitatorError;
use r402_protocol::payment::{PriceTag, ResourceInfo, SettleResponse, SettlementOverrides};
use r402_server::{
    AfterHandler, BackgroundSettlementTracker, CancellationGuard, CompletedSettlement,
    PaymentFlowName, ResourceServer, ScheduledSettlement, SequentialFinish, SettlePhase,
    SettlementMode, finish, run, schedule,
};
use tower::Service;

use super::fail::GateError;
use super::hooks::DynGateHooks;
use super::overrides::strip_response_settlement_overrides;
use super::receipt::{
    attach_failure_path_settlement, attach_payment_response, skip_handler_response,
};
use super::verify::{
    HANDLER_FAILED, VerifiedPayment, cancellation_guard, handler_failed_reason, map_settle,
    payment_flow_of, phases_of, settle_before_handler_if_needed,
};

/// HTTP payment gate.
pub struct Gate {
    pub(crate) server: ResourceServer,
    pub(crate) accepts: Arc<[PriceTag]>,
    pub(crate) resource: ResourceInfo,
    pub(crate) payment_required: Option<r402_protocol::payment::PaymentRequired>,
    pub(crate) hooks: Option<Arc<dyn DynGateHooks>>,
    pub(crate) settlement_mode: SettlementMode,
    pub(crate) settlement_tracker: Option<BackgroundSettlementTracker>,
    #[cfg(feature = "siwx")]
    pub(crate) siwx: Option<Arc<super::SiwxGate>>,
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

enum Prepared {
    Skipped(Response),
    Ready {
        verified: Box<VerifiedPayment>,
        before_handler: Option<Box<CompletedSettlement>>,
    },
}

#[derive(Debug)]
enum ConcurrentHandlerError {
    Failed(Box<Response>),
    SettlementAborted(String),
}

impl Gate {
    pub(crate) fn from_parts(
        server: ResourceServer,
        accepts: Vec<PriceTag>,
        resource: ResourceInfo,
        settlement_mode: SettlementMode,
        settlement_tracker: Option<BackgroundSettlementTracker>,
        hooks: Option<Arc<dyn DynGateHooks>>,
        #[cfg(feature = "siwx")] siwx: Option<Arc<super::SiwxGate>>,
    ) -> Self {
        Self {
            server,
            accepts: accepts.into(),
            resource,
            payment_required: None,
            hooks,
            settlement_mode,
            settlement_tracker,
            #[cfg(feature = "siwx")]
            siwx,
        }
    }

    /// Inserts a fresh SIWX challenge into the built 402 body.
    #[cfg(feature = "siwx")]
    pub(crate) fn attach_siwx_challenge(
        &mut self,
        siwx: &super::SiwxGate,
        path: &str,
    ) -> Result<(), GateError> {
        let Some(payment_required) = self.payment_required.as_mut() else {
            return Ok(());
        };
        let entry = siwx
            .challenge_entry(path)
            .map_err(|err| GateError::PaymentRequiredBuild(err.to_string()))?;
        payment_required.extensions.insert(super::SIWX_KEY, entry);
        Ok(())
    }

    fn record_siwx_success(&self, path: &str, settlement: &SettleResponse) {
        #[cfg(feature = "siwx")]
        if let Some(siwx) = self.siwx.as_ref() {
            record_paid_address(siwx, path, settlement);
        }
        #[cfg(not(feature = "siwx"))]
        {
            let _ = (path, settlement);
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
        let path = req.uri().path().to_owned();
        match self.prepare(&mut req).await? {
            Prepared::Skipped(response) => Ok(response),
            Prepared::Ready {
                verified,
                before_handler,
            } => {
                sequential_after_verify(
                    self,
                    &path,
                    inner,
                    req,
                    *verified,
                    before_handler.map(|b| *b),
                )
                .await
            }
        }
    }

    /// Concurrent: steps 1–5, then wrap 4xx/overrides as Err, then [`run`] `JoinSettle`.
    ///
    /// # Errors
    ///
    /// [`GateError`] for verify/settle/header failures. Overrides → 402
    /// [`GateError::SettlementAborted`].
    pub async fn handle_request_concurrent<S>(
        &self,
        inner: S,
        mut req: Request,
    ) -> Result<Response, GateError>
    where
        S: Service<Request, Response = Response, Error = Infallible> + Send,
        S::Future: Send,
    {
        let path = req.uri().path().to_owned();
        match self.prepare(&mut req).await? {
            Prepared::Skipped(response) => Ok(response),
            Prepared::Ready {
                verified,
                before_handler,
            } => {
                concurrent_after_verify(
                    self,
                    &path,
                    inner,
                    req,
                    *verified,
                    before_handler.map(|b| *b),
                )
                .await
            }
        }
    }

    /// Background: steps 1–5, then [`run`] `SpawnSettle`, strip overrides, no receipt.
    ///
    /// # Errors
    ///
    /// [`GateError`] for verify/header failures. Settlement errors are not returned.
    pub async fn handle_request_background<S>(
        &self,
        inner: S,
        mut req: Request,
    ) -> Result<Response, GateError>
    where
        S: Service<Request, Response = Response, Error = Infallible> + Send,
        S::Future: Send,
    {
        let path = req.uri().path().to_owned();
        match self.prepare(&mut req).await? {
            Prepared::Skipped(response) => Ok(response),
            Prepared::Ready {
                verified,
                before_handler,
            } => {
                background_after_verify(
                    self,
                    &path,
                    inner,
                    req,
                    *verified,
                    before_handler.map(|b| *b),
                )
                .await
            }
        }
    }

    async fn prepare(&self, req: &mut Request) -> Result<Prepared, GateError> {
        let verified = self.verify_only(req.headers()).await?;
        if let Some(h) = self.hooks.as_ref() {
            h.on_payment_verified(req).await;
        }
        if let Some(directive) = verified.skip_handler.clone() {
            let receipt = sequential_after_handler(&verified, None, None).await?;
            self.record_siwx_success(req.uri().path(), &receipt);
            return Ok(Prepared::Skipped(skip_handler_response(
                &directive, &receipt,
            )?));
        }
        let (flow, phases) = phases_of(&verified)?;
        let before_handler = settle_before_handler_if_needed(&verified, flow, phases).await?;
        Ok(Prepared::Ready {
            verified: Box::new(verified),
            before_handler: before_handler.map(Box::new),
        })
    }
}

async fn sequential_after_verify<S>(
    gate: &Gate,
    path: &str,
    inner: S,
    req: Request,
    verified: VerifiedPayment,
    before_handler: Option<CompletedSettlement>,
) -> Result<Response, GateError>
where
    S: Service<Request, Response = Response, Error = Infallible>,
    S::Future: Send,
{
    let cancel = cancellation_guard(&verified, before_handler.as_ref());
    let response = match call_inner(inner, req).await {
        Ok(r) => r,
        Err(never) => match never {},
    };

    if response.status().is_client_error() || response.status().is_server_error() {
        if let Some(completed) = before_handler.as_ref() {
            gate.record_siwx_success(path, &completed.result);
        }
        return failed_handler_response(
            response,
            &cancel,
            before_handler.as_ref(),
            Some(&verified.payload),
        )
        .await;
    }

    let mut response = response;
    let override_amount = super::overrides::resolve_response_settlement_amount(
        &mut response,
        verified.requirements(),
    )
    .map_err(|e| GateError::SettlementAborted(e.to_string()))?;
    let overrides = override_amount.as_deref().map(SettlementOverrides::amount);

    let settlement =
        sequential_after_handler(&verified, overrides.as_ref(), before_handler.as_ref()).await?;
    gate.record_siwx_success(path, &settlement);
    attach_payment_response(&mut response, &settlement)?;
    Ok(response)
}

async fn concurrent_after_verify<S>(
    gate: &Gate,
    path: &str,
    inner: S,
    req: Request,
    verified: VerifiedPayment,
    before_handler: Option<CompletedSettlement>,
) -> Result<Response, GateError>
where
    S: Service<Request, Response = Response, Error = Infallible> + Send,
    S::Future: Send,
{
    let (_, phases) = phases_of(&verified)?;
    let scheduled = schedule(phases, SettlementMode::Concurrent).map_err(|err| {
        GateError::IncompatibleSettlementMode {
            mode: err.mode,
            flow: err.flow,
        }
    })?;
    if scheduled.after_handler() != AfterHandler::JoinSettle {
        return Err(GateError::PaymentRequiredBuild(
            "concurrent handle_request cannot run sequential finish".into(),
        ));
    }

    let cancel = cancellation_guard(&verified, before_handler.as_ref());
    let settle = after_handler_settle(verified.clone());
    let outcome = run(scheduled, concurrent_handler(inner, req), settle).await;
    map_concurrent_outcome(
        gate,
        path,
        outcome,
        cancel,
        before_handler.as_ref(),
        Some(&verified.payload),
    )
    .await
}

async fn background_after_verify<S>(
    gate: &Gate,
    path: &str,
    inner: S,
    req: Request,
    verified: VerifiedPayment,
    before_handler: Option<CompletedSettlement>,
) -> Result<Response, GateError>
where
    S: Service<Request, Response = Response, Error = Infallible> + Send,
    S::Future: Send,
{
    let (_, phases) = phases_of(&verified)?;
    let scheduled = schedule(phases, SettlementMode::Background).map_err(|err| {
        GateError::IncompatibleSettlementMode {
            mode: err.mode,
            flow: err.flow,
        }
    })?;
    let scheduled = match gate.settlement_tracker.clone() {
        Some(tracker) => scheduled.with_tracker(tracker),
        None => scheduled,
    };
    if scheduled.after_handler() != AfterHandler::SpawnSettle {
        return Err(GateError::PaymentRequiredBuild(
            "background handle_request cannot run sequential finish".into(),
        ));
    }

    let cancel = cancellation_guard(&verified, before_handler.as_ref());
    let settle = wrap_background_siwx(gate, path, after_handler_settle(verified.clone()));
    let outcome = run(scheduled, background_handler(inner, req), settle).await;
    map_background_outcome(outcome, cancel).await
}

/// Spawned settle success is otherwise invisible to `PaidAddressStore`.
#[cfg(feature = "siwx")]
fn wrap_background_siwx(
    gate: &Gate,
    path: &str,
    settle: impl Future<Output = Result<SettleResponse, FacilitatorError>> + Send + 'static,
) -> Pin<Box<dyn Future<Output = Result<SettleResponse, FacilitatorError>> + Send>> {
    let siwx = gate.siwx.clone();
    let path = path.to_owned();
    Box::pin(async move {
        let result = settle.await;
        if let (Some(siwx), Ok(settlement)) = (siwx.as_ref(), result.as_ref()) {
            record_paid_address(siwx, &path, settlement);
        }
        result
    })
}

#[cfg(not(feature = "siwx"))]
fn wrap_background_siwx(
    _gate: &Gate,
    _path: &str,
    settle: impl Future<Output = Result<SettleResponse, FacilitatorError>> + Send + 'static,
) -> impl Future<Output = Result<SettleResponse, FacilitatorError>> + Send + 'static {
    settle
}

#[cfg(feature = "siwx")]
fn record_paid_address(siwx: &super::SiwxGate, path: &str, settlement: &SettleResponse) {
    let SettleResponse::Success { payer, .. } = settlement else {
        return;
    };
    let payer = payer.as_deref().unwrap_or("");
    if payer.is_empty() {
        return;
    }
    siwx.record_success(path, payer);
}

async fn after_handler_settle(
    verified: VerifiedPayment,
) -> Result<SettleResponse, FacilitatorError> {
    verified.settle_phase(SettlePhase::AfterHandler, None).await
}

async fn concurrent_handler<S>(inner: S, req: Request) -> Result<Response, ConcurrentHandlerError>
where
    S: Service<Request, Response = Response, Error = Infallible> + Send,
    S::Future: Send,
{
    let mut response = match call_inner(inner, req).await {
        Ok(r) => r,
        Err(never) => match never {},
    };
    if response.status().is_client_error() || response.status().is_server_error() {
        return Err(ConcurrentHandlerError::Failed(Box::new(response)));
    }
    // Partial settlement cannot apply: settle already runs at the signed max.
    if strip_response_settlement_overrides(&mut response) {
        return Err(ConcurrentHandlerError::SettlementAborted(
            "Settlement-Overrides / UptoActualAmount require SettlementMode::Sequential".into(),
        ));
    }
    Ok(response)
}

async fn background_handler<S>(inner: S, req: Request) -> Result<Response, Box<Response>>
where
    S: Service<Request, Response = Response, Error = Infallible> + Send,
    S::Future: Send,
{
    let response = match call_inner(inner, req).await {
        Ok(r) => r,
        Err(never) => match never {},
    };
    if response.status().is_client_error() || response.status().is_server_error() {
        Err(Box::new(response))
    } else {
        Ok(response)
    }
}

async fn map_concurrent_outcome(
    gate: &Gate,
    path: &str,
    outcome: ScheduledSettlement<Response, ConcurrentHandlerError>,
    cancel: CancellationGuard,
    before_handler: Option<&CompletedSettlement>,
    payload: Option<&r402_server::WirePaymentPayload>,
) -> Result<Response, GateError> {
    match outcome {
        ScheduledSettlement::HandlerOkSettleOk { mut value, receipt } => {
            let settlement = map_settle(Ok(*receipt))?;
            gate.record_siwx_success(path, &settlement);
            attach_payment_response(&mut value, &settlement)?;
            Ok(value)
        }
        ScheduledSettlement::HandlerErrDetach { error } => match error {
            ConcurrentHandlerError::SettlementAborted(detail) => {
                Err(GateError::SettlementAborted(detail))
            }
            ConcurrentHandlerError::Failed(response) => {
                failed_handler_response(*response, &cancel, before_handler, payload).await
            }
        },
        ScheduledSettlement::SettleErr { error, .. } => {
            Err(GateError::from_settle_facilitator(error))
        }
        ScheduledSettlement::Spawned { .. } => Err(GateError::PaymentRequiredBuild(
            "concurrent run returned Spawned".into(),
        )),
    }
}

async fn map_background_outcome(
    outcome: ScheduledSettlement<Response, Box<Response>>,
    cancel: CancellationGuard,
) -> Result<Response, GateError> {
    match outcome {
        ScheduledSettlement::HandlerErrDetach { mut error } => {
            let status = error.status().as_u16();
            let _ = cancel
                .cancel(handler_failed_reason(), Some(HANDLER_FAILED), Some(status))
                .await;
            let _ = strip_response_settlement_overrides(&mut error);
            Ok(*error)
        }
        ScheduledSettlement::Spawned { mut value }
        | ScheduledSettlement::HandlerOkSettleOk { mut value, .. }
        | ScheduledSettlement::SettleErr { mut value, .. } => {
            let _ = strip_response_settlement_overrides(&mut value);
            Ok(value)
        }
    }
}

async fn failed_handler_response(
    response: Response,
    cancel: &CancellationGuard,
    before_handler: Option<&CompletedSettlement>,
    payload: Option<&r402_server::WirePaymentPayload>,
) -> Result<Response, GateError> {
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
        before_handler,
        payload,
    )?;
    Ok(response)
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
            None::<std::future::Ready<Result<SettleResponse, FacilitatorError>>>,
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
        payer: None,
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
