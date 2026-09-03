#![allow(unused_crate_dependencies, reason = "sibling tests consume them")]
#![allow(
    clippy::tests_outside_test_module,
    reason = "integration test binaries put #[test] fns at file scope"
)]
#![allow(
    clippy::items_after_statements,
    clippy::let_underscore_must_use,
    clippy::missing_assert_message,
    clippy::missing_const_for_fn,
    clippy::panic,
    clippy::unwrap_used,
    reason = "idiomatic test-code patterns"
)]

//! Sequential `finish`; Concurrent/Background `run`. Compatibility in `settlement.rs`.

use std::collections::HashMap;
use std::future::Future;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use r402_facilitator::Facilitator;
use r402_protocol::error::FacilitatorError;
use r402_protocol::network::ChainIdPattern;
use r402_protocol::payment::{
    Extensions, PaymentRequirements, SettleRequest, SettleResponse, SupportedResponse,
    VerifyRequest, VerifyResponse,
};
use r402_server::{
    AfterHandler, BackgroundSettlementTracker, PaymentFlowConfig, PaymentFlowName, ResourceServer,
    SDK_DEFAULT_ASSET_TRANSFER_METHOD, ScheduledSettlement, SchemeNetworkServer, SequentialFinish,
    SettlePhase, SettlementMode, WirePaymentPayload, finish, run, schedule,
};

fn success(tx: &str) -> SettleResponse {
    SettleResponse::Success {
        payer: Some("0xpayer".into()),
        transaction: tx.into(),
        network: "eip155:8453".into(),
        amount: None,
        extensions: Extensions::new(),
        extension_responses: Extensions::new(),
        extra: None,
    }
}

fn auth() -> r402_server::PaymentFlowPhases {
    PaymentFlowName::Authorization.phases()
}

fn upfront() -> r402_server::PaymentFlowPhases {
    PaymentFlowName::Upfront.phases()
}

fn escrow() -> r402_server::PaymentFlowPhases {
    PaymentFlowName::Escrow.phases()
}

struct CountingFacilitator {
    settles: AtomicUsize,
}

impl Facilitator for CountingFacilitator {
    fn verify(
        &self,
        _request: VerifyRequest,
    ) -> impl Future<Output = Result<VerifyResponse, FacilitatorError>> + Send {
        std::future::ready(Ok(VerifyResponse::valid("0xpayer")))
    }

    fn settle(
        &self,
        _request: SettleRequest,
    ) -> impl Future<Output = Result<SettleResponse, FacilitatorError>> + Send {
        self.settles.fetch_add(1, Ordering::SeqCst);
        std::future::ready(Ok(success("0xtx")))
    }

    fn supported(
        &self,
    ) -> impl Future<Output = Result<SupportedResponse, FacilitatorError>> + Send {
        std::future::ready(Ok(SupportedResponse::default()))
    }
}

struct FlowScheme {
    flows: HashMap<String, PaymentFlowConfig>,
}

impl FlowScheme {
    fn new(flow: PaymentFlowName) -> Self {
        let mut flows = HashMap::new();
        flows.insert(
            SDK_DEFAULT_ASSET_TRANSFER_METHOD.into(),
            PaymentFlowConfig::new(vec![flow], flow),
        );
        Self { flows }
    }
}

impl SchemeNetworkServer for FlowScheme {
    fn scheme(&self) -> &'static str {
        "exact"
    }

    fn default_asset_transfer_method(&self) -> &'static str {
        SDK_DEFAULT_ASSET_TRANSFER_METHOD
    }

    fn payment_flows(&self) -> &HashMap<String, PaymentFlowConfig> {
        &self.flows
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

#[test]
fn sequential_authorization_is_wait_then_settle() {
    let schedule = schedule(auth(), SettlementMode::Sequential).unwrap();
    assert_eq!(schedule.after_handler(), AfterHandler::WaitThenSettle);
    assert!(schedule.attach_receipt());
}

#[test]
fn sequential_upfront_is_echo_receipt() {
    let schedule = schedule(upfront(), SettlementMode::Sequential).unwrap();
    assert_eq!(schedule.after_handler(), AfterHandler::EchoReceipt);
    assert!(schedule.attach_receipt());
}

#[test]
fn sequential_escrow_is_wait_then_settle() {
    let schedule = schedule(escrow(), SettlementMode::Sequential).unwrap();
    assert_eq!(schedule.after_handler(), AfterHandler::WaitThenSettle);
}

#[test]
fn concurrent_and_background_authorization_are_legal() {
    let concurrent = schedule(auth(), SettlementMode::Concurrent).unwrap();
    assert_eq!(concurrent.after_handler(), AfterHandler::JoinSettle);
    assert!(concurrent.attach_receipt());
    let background = schedule(auth(), SettlementMode::Background).unwrap();
    assert_eq!(background.after_handler(), AfterHandler::SpawnSettle);
    assert!(!background.attach_receipt());
}

#[test]
fn concurrent_upfront_is_incompatible() {
    let err = schedule(upfront(), SettlementMode::Concurrent).unwrap_err();
    assert_eq!(err.mode, SettlementMode::Concurrent);
    assert_eq!(err.flow, PaymentFlowName::Upfront);
}

#[test]
fn background_escrow_is_incompatible() {
    let err = schedule(escrow(), SettlementMode::Background).unwrap_err();
    assert_eq!(err.mode, SettlementMode::Background);
    assert_eq!(err.flow, PaymentFlowName::Escrow);
}

#[test]
fn concurrent_escrow_is_incompatible() {
    let err = schedule(escrow(), SettlementMode::Concurrent).unwrap_err();
    assert_eq!(err.flow, PaymentFlowName::Escrow);
}

#[tokio::test]
async fn finish_wait_then_settle_awaits_receipt() {
    let schedule = schedule(auth(), SettlementMode::Sequential).unwrap();
    let got = finish(schedule, Some(async { Ok(success("0xseq")) }))
        .await
        .unwrap();
    assert!(matches!(
        got,
        SequentialFinish::Settled(SettleResponse::Success { ref transaction, .. })
            if transaction == "0xseq"
    ));
}

#[tokio::test]
async fn finish_wait_then_settle_propagates_error() {
    let schedule = schedule(auth(), SettlementMode::Sequential).unwrap();
    let err = finish(
        schedule,
        Some(async { Err(FacilitatorError::Onchain("boom".into())) }),
    )
    .await
    .unwrap_err();
    assert!(matches!(err, FacilitatorError::Onchain(_)));
}

#[tokio::test]
async fn finish_echo_receipt_takes_none() {
    let schedule = schedule(upfront(), SettlementMode::Sequential).unwrap();
    let got = finish(
        schedule,
        None::<std::future::Ready<Result<SettleResponse, FacilitatorError>>>,
    )
    .await
    .unwrap();
    assert_eq!(got, SequentialFinish::Echo);
}

#[tokio::test]
async fn sequential_upfront_settles_once_then_echo() {
    let mock = Arc::new(CountingFacilitator {
        settles: AtomicUsize::new(0),
    });
    let rs = ResourceServer::new(Arc::clone(&mock)).with_scheme(
        ChainIdPattern::wildcard("eip155"),
        FlowScheme::new(PaymentFlowName::Upfront),
    );
    let payload = sample_payload();
    let req = payload.accepted.clone();
    assert!(
        rs.settle_payment(&payload, &req, None, SettlePhase::BeforeHandler)
            .await
            .unwrap()
            .is_success()
    );
    let schedule = schedule(upfront(), SettlementMode::Sequential).unwrap();
    let got = finish(
        schedule,
        None::<std::future::Ready<Result<SettleResponse, FacilitatorError>>>,
    )
    .await
    .unwrap();
    assert_eq!(got, SequentialFinish::Echo);
    assert_eq!(
        mock.settles.load(Ordering::SeqCst),
        1,
        "upfront settle-before only; EchoReceipt must not settle again"
    );
}

#[tokio::test]
async fn sequential_escrow_settles_before_and_after() {
    let mock = Arc::new(CountingFacilitator {
        settles: AtomicUsize::new(0),
    });
    let rs = ResourceServer::new(Arc::clone(&mock)).with_scheme(
        ChainIdPattern::wildcard("eip155"),
        FlowScheme::new(PaymentFlowName::Escrow),
    );
    let payload = sample_payload();
    let req = payload.accepted.clone();
    assert!(
        rs.settle_payment(&payload, &req, None, SettlePhase::BeforeHandler)
            .await
            .unwrap()
            .is_success()
    );
    let schedule = schedule(escrow(), SettlementMode::Sequential).unwrap();
    let rs_after = rs.clone();
    let payload_after = payload.clone();
    let req_after = req.clone();
    let got = finish(
        schedule,
        Some(async move {
            rs_after
                .settle_payment(&payload_after, &req_after, None, SettlePhase::AfterHandler)
                .await
        }),
    )
    .await
    .unwrap();
    assert!(matches!(got, SequentialFinish::Settled(_)));
    assert_eq!(
        mock.settles.load(Ordering::SeqCst),
        2,
        "escrow settle-before plus finish WaitThenSettle"
    );
}

#[tokio::test]
async fn finish_rejects_concurrent_schedule() {
    let schedule = schedule(auth(), SettlementMode::Concurrent).unwrap();
    let err = finish(schedule, Some(async { Ok(success("0x")) }))
        .await
        .unwrap_err();
    assert!(matches!(err, FacilitatorError::Internal(_)));
}

#[tokio::test]
async fn run_join_settle_ok() {
    let schedule = schedule(auth(), SettlementMode::Concurrent).unwrap();
    let out = run(schedule, async { Ok::<_, &str>(7) }, async {
        Ok(success("0xjoin"))
    })
    .await;
    match out {
        ScheduledSettlement::HandlerOkSettleOk { value, receipt } => {
            assert_eq!(value, 7);
            assert!(matches!(
                receipt,
                SettleResponse::Success { ref transaction, .. } if transaction == "0xjoin"
            ));
        }
        other => panic!("expected join ok, got {other:?}"),
    }
}

#[tokio::test]
async fn run_join_settle_handler_err_detaches() {
    let (tx, rx) = tokio::sync::oneshot::channel::<()>();
    let settle = async move {
        let _ = rx.await;
        Ok(success("0xlater"))
    };
    let schedule = schedule(auth(), SettlementMode::Concurrent).unwrap();
    let out = run(schedule, async { Err::<(), _>("handler") }, settle).await;
    assert!(matches!(
        out,
        ScheduledSettlement::HandlerErrDetach { error: "handler" }
    ));
    assert!(
        !tx.is_closed(),
        "dropping the join handle detaches settle; it must not abort"
    );
}

#[tokio::test]
async fn run_join_settle_err() {
    let schedule = schedule(auth(), SettlementMode::Concurrent).unwrap();
    let out = run(schedule, async { Ok::<_, &str>(1) }, async {
        Err(FacilitatorError::Onchain("settle".into()))
    })
    .await;
    match out {
        ScheduledSettlement::SettleErr { value, error } => {
            assert_eq!(value, 1);
            assert!(matches!(error, FacilitatorError::Onchain(_)));
        }
        other => panic!("expected settle err, got {other:?}"),
    }
}

#[tokio::test]
async fn run_spawn_settle_returns_without_waiting() {
    let tracker = BackgroundSettlementTracker::new();
    let (tx, rx) = tokio::sync::oneshot::channel::<()>();
    let settle = async move {
        let _ = rx.await;
        Ok(success("0xbg"))
    };
    let schedule = schedule(auth(), SettlementMode::Background)
        .unwrap()
        .with_tracker(tracker.clone());
    let out = run(schedule, async { Ok::<_, &str>(9) }, settle).await;
    assert!(matches!(out, ScheduledSettlement::Spawned { value: 9 }));
    assert!(tracker.in_flight() >= 1);
    tx.send(()).unwrap();
    tracker
        .wait_for_drain(Duration::from_secs(1))
        .await
        .expect("background settle drains");
}

#[tokio::test]
async fn run_does_not_execute_sequential_as_wait_then_settle() {
    let schedule = schedule(auth(), SettlementMode::Sequential).unwrap();
    let out = run(schedule, async { Ok::<_, &str>(1) }, async {
        Ok(success("0xseq"))
    })
    .await;
    assert!(matches!(
        out,
        ScheduledSettlement::SettleErr { value: 1, .. }
    ));
}
