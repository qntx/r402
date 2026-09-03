//! `SettlementMode` scheduler: when after-handler settle runs relative to the handler.
//!
//! Orthogonal to [`crate::PaymentFlowName`]. Concurrent and Background are
//! illegal when the flow settles before the handler (upfront / escrow).

mod tracker;

use std::fmt::{self, Display, Formatter};
use std::future::Future;

use r402_protocol::error::FacilitatorError;
use r402_protocol::payment::SettleResponse;
pub use tracker::BackgroundSettlementTracker;

use crate::payment_flow::{PaymentFlowName, PaymentFlowPhases};

/// When on-chain settlement runs relative to the resource handler (authorization after-handler).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum SettlementMode {
    /// Settlement runs after the handler completes.
    #[default]
    Sequential,
    /// Settlement runs concurrently with the handler; the caller waits for both.
    Concurrent,
    /// Settlement is fire-and-forget; the caller does not wait or attach a receipt.
    Background,
}

impl SettlementMode {
    /// Stable lowercase name.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Sequential => "sequential",
            Self::Concurrent => "concurrent",
            Self::Background => "background",
        }
    }
}

impl Display for SettlementMode {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// After-handler action selected by [`schedule`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AfterHandler {
    /// Sequential: await settle after the handler (authorization / escrow).
    WaitThenSettle,
    /// Concurrent: join the in-flight settle after the handler succeeds.
    JoinSettle,
    /// Background: settle already spawned; do not await.
    SpawnSettle,
    /// Sequential upfront: no after-handler settle.
    EchoReceipt,
}

/// Concurrent or Background used with a flow that settles before the handler.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("incompatible settlement mode {mode} with payment flow {flow}")]
pub struct IncompatibleSettlementMode {
    /// Requested settlement mode.
    pub mode: SettlementMode,
    /// Payment flow that cannot use that mode.
    pub flow: PaymentFlowName,
}

/// Result of [`schedule`].
#[derive(Debug, Clone)]
pub struct SettlementSchedule {
    mode: SettlementMode,
    phases: PaymentFlowPhases,
    after_handler: AfterHandler,
    attach_receipt: bool,
    tracker: Option<BackgroundSettlementTracker>,
}

impl SettlementSchedule {
    /// After-handler action.
    #[must_use]
    pub const fn after_handler(&self) -> AfterHandler {
        self.after_handler
    }

    /// Whether a `Payment-Response` receipt should be attached (false for Background).
    #[must_use]
    pub const fn attach_receipt(&self) -> bool {
        self.attach_receipt
    }

    /// Settlement mode this schedule was built from.
    #[must_use]
    pub const fn mode(&self) -> SettlementMode {
        self.mode
    }

    /// Payment-flow phases this schedule was built from.
    #[must_use]
    pub const fn phases(&self) -> PaymentFlowPhases {
        self.phases
    }

    /// Attaches a background in-flight tracker (used by [`run`] for [`AfterHandler::SpawnSettle`]).
    #[must_use]
    pub fn with_tracker(mut self, tracker: BackgroundSettlementTracker) -> Self {
        self.tracker = Some(tracker);
        self
    }
}

/// Outcome of [`finish`] (Sequential).
///
/// [`Self::Echo`] exists so a caller cannot treat a missing after-handler
/// settle as a new [`SettleResponse`] to attach.
#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(
    clippy::large_enum_variant,
    reason = "Settled is the sequential success path; Echo is a zero-sized marker"
)]
#[must_use]
pub enum SequentialFinish {
    /// After-handler settle completed (authorization / escrow). Attach this receipt.
    Settled(SettleResponse),
    /// No after-handler settle (upfront). Attach the before-handler receipt.
    Echo,
}

/// Outcome of [`run`] (Concurrent / Background).
#[derive(Debug)]
#[must_use]
pub enum ScheduledSettlement<T, E> {
    /// Handler and settle both succeeded (Concurrent join).
    HandlerOkSettleOk {
        /// Handler value.
        value: T,
        /// Settle receipt.
        receipt: SettleResponse,
    },
    /// Handler failed. Concurrent: settle handle dropped (detached). Background: settle already spawned.
    HandlerErrDetach {
        /// Handler error.
        error: E,
    },
    /// Handler succeeded; settle failed (Concurrent join).
    SettleErr {
        /// Handler value.
        value: T,
        /// Settle error.
        error: FacilitatorError,
    },
    /// Background: handler succeeded; settle is running detached.
    Spawned {
        /// Handler value.
        value: T,
    },
}

/// Selects the after-handler settle action for one resolved flow × mode pair.
///
/// # Errors
///
/// [`IncompatibleSettlementMode`] when `mode` is Concurrent or Background and
/// `flow` settles before the handler (upfront / escrow).
pub fn schedule(
    flow: PaymentFlowPhases,
    mode: SettlementMode,
) -> Result<SettlementSchedule, IncompatibleSettlementMode> {
    if mode != SettlementMode::Sequential && flow.settle_before_handler {
        return Err(IncompatibleSettlementMode {
            mode,
            flow: flow_name(flow),
        });
    }
    let after_handler = match mode {
        SettlementMode::Sequential => {
            if flow.settle_after_handler {
                AfterHandler::WaitThenSettle
            } else {
                AfterHandler::EchoReceipt
            }
        }
        SettlementMode::Concurrent => AfterHandler::JoinSettle,
        SettlementMode::Background => AfterHandler::SpawnSettle,
    };
    Ok(SettlementSchedule {
        mode,
        phases: flow,
        after_handler,
        attach_receipt: mode != SettlementMode::Background,
        tracker: None,
    })
}

fn flow_name(phases: PaymentFlowPhases) -> PaymentFlowName {
    for (name, table) in crate::PAYMENT_FLOWS {
        if table == phases {
            return name;
        }
    }
    if phases.settle_before_handler {
        if phases.settle_after_handler {
            PaymentFlowName::Escrow
        } else {
            PaymentFlowName::Upfront
        }
    } else {
        PaymentFlowName::Authorization
    }
}

/// Sequential after-handler settle. Does not take a handler.
///
/// [`AfterHandler::WaitThenSettle`] awaits `settle`. [`AfterHandler::EchoReceipt`]
/// requires `settle` to be `None` and returns [`SequentialFinish::Echo`].
///
/// # Errors
///
/// Propagates facilitator errors from `settle`. Returns
/// [`FacilitatorError::Internal`] when `schedule` is not sequential or the
/// `settle` option does not match [`AfterHandler`].
pub async fn finish<S>(
    schedule: SettlementSchedule,
    settle: Option<S>,
) -> Result<SequentialFinish, FacilitatorError>
where
    S: Future<Output = Result<SettleResponse, FacilitatorError>> + Send,
{
    match schedule.after_handler {
        AfterHandler::WaitThenSettle => {
            let fut = settle.ok_or_else(|| {
                FacilitatorError::internal("WaitThenSettle requires an after-handler settle")
            })?;
            Ok(SequentialFinish::Settled(fut.await?))
        }
        AfterHandler::EchoReceipt => {
            if settle.is_some() {
                return Err(FacilitatorError::internal(
                    "EchoReceipt does not take an after-handler settle",
                ));
            }
            Ok(SequentialFinish::Echo)
        }
        AfterHandler::JoinSettle | AfterHandler::SpawnSettle => {
            Err(FacilitatorError::internal("finish is sequential-only"))
        }
    }
}

/// Concurrent / Background after-handler settle. Does not inspect transport status.
///
/// Sequential schedules are not executed as wait-then-settle; call [`finish`].
pub async fn run<T, E, H, S>(
    schedule: SettlementSchedule,
    handler: H,
    settle: S,
) -> ScheduledSettlement<T, E>
where
    T: Send,
    E: Send,
    H: Future<Output = Result<T, E>> + Send,
    S: Future<Output = Result<SettleResponse, FacilitatorError>> + Send + 'static,
{
    match schedule.after_handler {
        AfterHandler::JoinSettle => join_settle(handler, settle).await,
        AfterHandler::SpawnSettle => spawn_settle(schedule.tracker, handler, settle).await,
        AfterHandler::WaitThenSettle | AfterHandler::EchoReceipt => {
            drop(settle);
            match handler.await {
                Ok(value) => ScheduledSettlement::SettleErr {
                    value,
                    error: FacilitatorError::internal("run is concurrent/background"),
                },
                Err(error) => ScheduledSettlement::HandlerErrDetach { error },
            }
        }
    }
}

async fn join_settle<T, E, H, S>(handler: H, settle: S) -> ScheduledSettlement<T, E>
where
    H: Future<Output = Result<T, E>> + Send,
    S: Future<Output = Result<SettleResponse, FacilitatorError>> + Send + 'static,
{
    let settle_handle = tokio::spawn(settle);
    match handler.await {
        Ok(value) => match settle_handle.await {
            Ok(Ok(receipt)) => ScheduledSettlement::HandlerOkSettleOk { value, receipt },
            Ok(Err(error)) => ScheduledSettlement::SettleErr { value, error },
            Err(join) => ScheduledSettlement::SettleErr {
                value,
                error: FacilitatorError::internal(join),
            },
        },
        Err(error) => {
            drop(settle_handle);
            ScheduledSettlement::HandlerErrDetach { error }
        }
    }
}

async fn spawn_settle<T, E, H, S>(
    tracker: Option<BackgroundSettlementTracker>,
    handler: H,
    settle: S,
) -> ScheduledSettlement<T, E>
where
    H: Future<Output = Result<T, E>> + Send,
    S: Future<Output = Result<SettleResponse, FacilitatorError>> + Send + 'static,
{
    let settle_handle = tokio::spawn(settle);
    let tracker_guard = tracker.as_ref().map(BackgroundSettlementTracker::start);
    drop(tokio::spawn(supervise_background_settle(
        settle_handle,
        tracker_guard,
    )));
    match handler.await {
        Ok(value) => ScheduledSettlement::Spawned { value },
        Err(error) => ScheduledSettlement::HandlerErrDetach { error },
    }
}

async fn supervise_background_settle(
    handle: tokio::task::JoinHandle<Result<SettleResponse, FacilitatorError>>,
    _tracker: Option<tracker::SettlementInFlightGuard>,
) {
    let outcome = handle.await;
    log_background_settle_outcome(&outcome);
    record_background_settle_metric(&outcome);
}

fn log_background_settle_outcome(
    outcome: &Result<Result<SettleResponse, FacilitatorError>, tokio::task::JoinError>,
) {
    match outcome {
        Ok(Ok(_)) => {
            #[cfg(feature = "telemetry")]
            tracing::debug!("background settlement completed");
        }
        Ok(Err(err)) => {
            #[cfg(feature = "telemetry")]
            tracing::error!(error = %err, "background settlement returned error");
            #[cfg(not(feature = "telemetry"))]
            let _ = err;
        }
        Err(join_err) => {
            #[cfg(feature = "telemetry")]
            if join_err.is_panic() {
                tracing::error!(error = %join_err, "background settlement task panicked");
            } else {
                tracing::warn!(error = %join_err, "background settlement task cancelled");
            }
            #[cfg(not(feature = "telemetry"))]
            let _ = join_err;
        }
    }
}

fn record_background_settle_metric(
    outcome: &Result<Result<SettleResponse, FacilitatorError>, tokio::task::JoinError>,
) {
    #[cfg(feature = "metrics")]
    {
        let result = match outcome {
            Ok(Ok(_)) => "ok",
            Ok(Err(_)) => "error",
            Err(join_err) if join_err.is_panic() => "panic",
            Err(_) => "cancelled",
        };
        ::metrics::counter!(
            r402_protocol::metrics::BACKGROUND_SETTLE_TOTAL,
            "result" => result,
        )
        .increment(1);
    }
    #[cfg(not(feature = "metrics"))]
    let _ = outcome;
}
