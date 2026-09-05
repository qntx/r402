#![allow(unused_crate_dependencies, reason = "sibling tests consume them")]
#![allow(
    clippy::tests_outside_test_module,
    reason = "integration test binaries put #[test] fns at file scope"
)]
#![allow(
    clippy::indexing_slicing,
    clippy::items_after_statements,
    clippy::missing_assert_message,
    clippy::missing_const_for_fn,
    clippy::panic,
    clippy::unwrap_used,
    clippy::manual_async_fn,
    clippy::unused_async,
    reason = "idiomatic test-code patterns"
)]

//! Resource-server hooks, matching, cancel, and verify/settle orchestration.

use std::collections::HashMap;
use std::future::Future;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use r402_facilitator::{Facilitator, FailureRecovery};
use r402_protocol::error::{ErrorReason, FacilitatorError, VerificationError};
use r402_protocol::extension::{AdvertiseContext, Extension, SettleContext as ExtSettleContext};
use r402_protocol::network::ChainIdPattern;
use r402_protocol::payment::{
    ExtensionEntry, Extensions, PaymentRequirements, ResourceInfo, SettleRequest, SettleResponse,
    SupportedResponse, VerifyRequest, VerifyResponse,
};
use r402_server::{
    AfterVerifyDecision, BeforeOpDecision, CancelReason, CompletedSettlement, PaymentFlowConfig,
    PaymentFlowError, PaymentFlowName, PaymentHookContext, PaymentRequiredBuildContext,
    ResourceServer, ResourceServerHooks, SDK_DEFAULT_ASSET_TRANSFER_METHOD, SchemeNetworkServer,
    SettleContext, SettlePhase, SkipHandlerDirective, VerifiedPaymentCanceledContext,
    VerifyResultContext, WirePaymentPayload, build_failure_path_settlement_response,
    is_authorization_payment_flow,
};
use serde_json::{Map, Value};

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
            Ok(success_settle("0xtx"))
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

fn success_settle(transaction: &str) -> SettleResponse {
    SettleResponse::Success {
        payer: Some("0xpayer".into()),
        transaction: transaction.into(),
        network: "eip155:8453".into(),
        amount: None,
        extensions: Extensions::new(),
        extension_responses: Extensions::new(),
        extra: None,
    }
}

fn eip155_wildcard() -> ChainIdPattern {
    ChainIdPattern::wildcard("eip155")
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

    fn settles_on_cancel(&self) -> bool {
        self.cancel_requirements.is_some()
    }

    fn settle_on_cancel<'a>(
        &'a self,
        _: &'a VerifiedPaymentCanceledContext,
    ) -> impl Future<Output = Option<PaymentRequirements>> + Send + 'a {
        std::future::ready(self.cancel_requirements.clone())
    }
}

#[tokio::test]
async fn verify_and_settle_invoke_facilitator() {
    let mock = mock_ok();
    let rs = ResourceServer::new(Arc::clone(&mock))
        .with_scheme(eip155_wildcard(), MockScheme::authorization());
    let payload = sample_payload();
    let req = payload.accepted.clone();
    let out = rs.verify_payment(&payload, &req, None).await.unwrap();
    assert!(out.response.is_valid());
    assert!(out.skip_handler.is_none());
    assert!(
        rs.settle_payment(&payload, &req, None, SettlePhase::AfterHandler, None, None)
            .await
            .unwrap()
            .is_success()
    );
    assert_eq!(mock.verifies.load(Ordering::SeqCst), 1);
    assert_eq!(mock.settles.load(Ordering::SeqCst), 1);
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
    let err = rs.verify_payment(&payload, &req, None).await.unwrap_err();
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
    let out = rs.verify_payment(&payload, &req, None).await.unwrap();
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
    let err = rs.verify_payment(&payload, &req, None).await.unwrap_err();
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
    let out = rs.verify_payment(&payload, &req, None).await.unwrap();
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
        rs.verify_payment(&payload, &req, None)
            .await
            .unwrap()
            .response
            .is_valid()
    );
}

#[tokio::test]
async fn cancellation_guard_fires_once() {
    let count = Arc::new(AtomicUsize::new(0));
    struct CancelCountHook(Arc<AtomicUsize>);
    impl ResourceServerHooks for CancelCountHook {
        fn on_verified_payment_canceled<'a>(
            &'a self,
            _: &'a VerifiedPaymentCanceledContext,
        ) -> impl Future<Output = ()> + Send + 'a {
            self.0.fetch_add(1, Ordering::SeqCst);
            std::future::ready(())
        }
    }
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

#[tokio::test]
async fn verify_skips_facilitator_when_not_verify_before_handler() {
    let mock = mock_ok();
    let rs = ResourceServer::new(Arc::clone(&mock))
        .with_scheme(eip155_wildcard(), MockScheme::upfront());
    let payload = sample_payload();
    let req = payload.accepted.clone();
    let out = rs.verify_payment(&payload, &req, None).await.unwrap();
    assert!(out.response.is_valid());
    assert_eq!(mock.verifies.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn verify_payment_errors_when_table_empty() {
    let mock = mock_ok();
    let rs = ResourceServer::new(Arc::clone(&mock));
    let payload = sample_payload();
    let req = payload.accepted.clone();
    let err = rs.verify_payment(&payload, &req, None).await.unwrap_err();
    assert!(matches!(
        err,
        FacilitatorError::Verification(VerificationError::InvalidFormat(ref msg))
            if msg.contains("No server implementation registered")
    ));
    assert_eq!(mock.verifies.load(Ordering::SeqCst), 0);
}

#[test]
fn get_payment_flow_resolves_registered_scheme() {
    let rs = ResourceServer::new(mock_ok()).with_scheme(eip155_wildcard(), MockScheme::upfront());
    assert_eq!(
        rs.get_payment_flow(&sample_payload().accepted).unwrap(),
        PaymentFlowName::Upfront
    );
}

#[test]
fn get_payment_flow_errors_when_table_empty() {
    let rs = ResourceServer::new(mock_ok());
    let err = rs.get_payment_flow(&sample_payload().accepted).unwrap_err();
    assert!(matches!(err, PaymentFlowError::UnregisteredScheme { .. }));
}

struct QueueFacilitator {
    settles: Mutex<std::collections::VecDeque<Result<SettleResponse, FacilitatorError>>>,
    settle_calls: AtomicUsize,
}

impl QueueFacilitator {
    fn new(responses: Vec<Result<SettleResponse, FacilitatorError>>) -> Arc<Self> {
        Arc::new(Self {
            settles: Mutex::new(responses.into()),
            settle_calls: AtomicUsize::new(0),
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
        _request: SettleRequest,
    ) -> impl Future<Output = Result<SettleResponse, FacilitatorError>> + Send {
        self.settle_calls.fetch_add(1, Ordering::SeqCst);
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

fn pending_failure(transaction: &str) -> SettleResponse {
    SettleResponse::Failure {
        reason: ErrorReason::SettlementPending,
        message: None,
        payer: None,
        transaction: transaction.into(),
        network: "eip155:8453".into(),
        extensions: Extensions::new(),
        extension_responses: Extensions::new(),
        extra: None,
    }
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
        .settle_payment(&payload, &req, None, SettlePhase::AfterHandler, None, None)
        .await
        .unwrap();
    assert!(matches!(
        result,
        SettleResponse::Success { ref transaction, .. } if transaction == "0xconfirmed"
    ));
    assert_eq!(mock.settle_calls.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn settle_does_not_retry_on_facilitator_error() {
    let mock = QueueFacilitator::new(vec![Err(FacilitatorError::Onchain("rpc timeout".into()))]);
    let rs = ResourceServer::new(Arc::clone(&mock));
    let payload = sample_payload();
    let req = payload.accepted.clone();
    let err = rs
        .settle_payment(&payload, &req, None, SettlePhase::AfterHandler, None, None)
        .await
        .unwrap_err();
    assert!(matches!(err, FacilitatorError::Onchain(_)));
    assert_eq!(mock.settle_calls.load(Ordering::SeqCst), 1);
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
    ) -> impl Future<Output = Result<Option<Map<String, Value>>, FacilitatorError>> + Send + 'a
    {
        let enrichment = match ctx.phase {
            SettlePhase::BeforeHandler => None,
            SettlePhase::AfterHandler | SettlePhase::Cancel => {
                let mut map = Map::new();
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

    rs.settle_payment(&payload, &req, None, SettlePhase::BeforeHandler, None, None)
        .await
        .unwrap();
    rs.settle_payment(&payload, &req, None, SettlePhase::AfterHandler, None, None)
        .await
        .unwrap();
    rs.settle_payment(&payload, &req, None, SettlePhase::Cancel, None, None)
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

#[tokio::test]
async fn cancel_settles_when_before_handler_and_settle_on_cancel() {
    let mock = mock_ok();
    let payload = sample_payload();
    let req = payload.accepted.clone();
    let mut scheme = MockScheme::escrow();
    scheme.cancel_requirements = Some(req.clone());
    let rs = ResourceServer::new(Arc::clone(&mock)).with_scheme(eip155_wildcard(), scheme);
    let guard = rs
        .cancellation_guard(payload, req)
        .with_settled_phases([SettlePhase::BeforeHandler]);
    let receipt = guard
        .cancel(CancelReason::HandlerFailed, Some("4xx"), Some(400))
        .await;
    assert!(receipt.unwrap().is_success());
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

#[tokio::test]
async fn create_payment_required_omits_authorization_payment_flow() {
    let rs =
        ResourceServer::new(mock_ok()).with_scheme(eip155_wildcard(), MockScheme::authorization());
    let built = rs
        .create_payment_required_response(
            vec![sample_payload().accepted],
            PaymentRequiredBuildContext {
                resource: ResourceInfo::new("https://example.com/paid"),
                error: None,
                extensions: Extensions::new(),
                supported: SupportedResponse::default(),
                payment_payload: None,
            },
        )
        .await
        .unwrap();
    let extra = built
        .accepts
        .first()
        .and_then(|accept| accept.extra.as_ref());
    assert!(extra.and_then(|value| value.get("paymentFlow")).is_none());
    assert!(is_authorization_payment_flow(extra));
}

fn upto_accept(network: &str) -> PaymentRequirements {
    PaymentRequirements::new(
        "upto".into(),
        network.parse().unwrap(),
        "1".into(),
        "0xa".into(),
        "0xb".into(),
        60,
    )
}

fn insert_fee_payer(req: &mut PaymentRequirements) -> bool {
    if req
        .extra
        .as_ref()
        .and_then(Value::as_object)
        .is_some_and(|extra| extra.contains_key("feePayer"))
    {
        return false;
    }
    let mut extra = req
        .extra
        .take()
        .and_then(|value| value.as_object().cloned())
        .unwrap_or_default();
    extra.insert("feePayer".into(), Value::String("0xfee".into()));
    req.extra = Some(Value::Object(extra));
    true
}

/// Writes `feePayer` only on the accept bound to this enrich invocation.
struct BoundNetworkUpto;

impl SchemeNetworkServer for BoundNetworkUpto {
    fn scheme(&self) -> &'static str {
        "upto"
    }

    fn default_asset_transfer_method(&self) -> &'static str {
        "permit2"
    }

    fn payment_flows(&self) -> &HashMap<String, PaymentFlowConfig> {
        static FLOWS: std::sync::LazyLock<HashMap<String, PaymentFlowConfig>> =
            std::sync::LazyLock::new(|| {
                HashMap::from([("permit2".into(), PaymentFlowConfig::authorization_only())])
            });
        &FLOWS
    }

    fn enrich_payment_required_response<'a>(
        &'a self,
        ctx: &'a r402_server::SchemePaymentRequiredContext<'a>,
    ) -> impl Future<Output = Option<Vec<PaymentRequirements>>> + Send + 'a {
        let mut accepts = ctx.requirements.to_vec();
        let changed = accepts.iter_mut().fold(false, |acc, req| {
            acc | (req.scheme.as_str() == "upto"
                && req.network == *ctx.network
                && insert_fee_payer(req))
        });
        std::future::ready(changed.then_some(accepts))
    }
}

/// Writes `feePayer` onto every same-scheme accept (the pre-fix bug).
struct CrossNetworkUpto;

impl SchemeNetworkServer for CrossNetworkUpto {
    fn scheme(&self) -> &'static str {
        "upto"
    }

    fn default_asset_transfer_method(&self) -> &'static str {
        "permit2"
    }

    fn payment_flows(&self) -> &HashMap<String, PaymentFlowConfig> {
        static FLOWS: std::sync::LazyLock<HashMap<String, PaymentFlowConfig>> =
            std::sync::LazyLock::new(|| {
                HashMap::from([("permit2".into(), PaymentFlowConfig::authorization_only())])
            });
        &FLOWS
    }

    fn enrich_payment_required_response<'a>(
        &'a self,
        ctx: &'a r402_server::SchemePaymentRequiredContext<'a>,
    ) -> impl Future<Output = Option<Vec<PaymentRequirements>>> + Send + 'a {
        let mut accepts = ctx.requirements.to_vec();
        let changed = accepts.iter_mut().fold(false, |acc, req| {
            acc | (req.scheme.as_str() == "upto" && insert_fee_payer(req))
        });
        std::future::ready(changed.then_some(accepts))
    }
}

fn two_network_upto_accepts() -> Vec<PaymentRequirements> {
    vec![upto_accept("eip155:143"), upto_accept("eip155:10143")]
}

#[tokio::test]
async fn create_payment_required_two_networks_enriches_each_row() {
    let rs = ResourceServer::new(mock_ok()).with_scheme(eip155_wildcard(), BoundNetworkUpto);
    let built = rs
        .create_payment_required_response(
            two_network_upto_accepts(),
            PaymentRequiredBuildContext {
                resource: ResourceInfo::new("https://example.com/paid"),
                error: None,
                extensions: Extensions::new(),
                supported: SupportedResponse::default(),
                payment_payload: None,
            },
        )
        .await
        .expect("bound enrich must not trip accepts mutation policy");
    assert_eq!(built.accepts.len(), 2);
    for accept in &built.accepts {
        let extra = accept.extra.as_ref().and_then(Value::as_object);
        assert_eq!(
            extra
                .and_then(|map| map.get("feePayer"))
                .and_then(Value::as_str),
            Some("0xfee"),
            "each network gets its own enrich pass"
        );
    }
}

#[tokio::test]
async fn create_payment_required_rejects_cross_network_enrich() {
    let rs = ResourceServer::new(mock_ok()).with_scheme(eip155_wildcard(), CrossNetworkUpto);
    let err = rs
        .create_payment_required_response(
            two_network_upto_accepts(),
            PaymentRequiredBuildContext {
                resource: ResourceInfo::new("https://example.com/paid"),
                error: None,
                extensions: Extensions::new(),
                supported: SupportedResponse::default(),
                payment_payload: None,
            },
        )
        .await
        .expect_err("writing extra onto the unbound network must fail closed");
    let msg = err.to_string();
    assert!(msg.contains("only matching accepts"), "{msg}");
    assert!(msg.contains("index 1"), "{msg}");
}

struct OfferStub;

impl Extension for OfferStub {
    fn id(&self) -> &'static str {
        "offer-receipt"
    }

    fn dynamic_info_fields(&self) -> &'static [&'static str] {
        &["offers"]
    }

    fn enrich_payment_required<'a>(
        &'a self,
        ctx: &'a AdvertiseContext<'a>,
    ) -> impl Future<Output = Option<ExtensionEntry>> + Send + 'a {
        let url = ctx.resource.map(|r| r.url.to_string());
        let n = ctx.accepts.len();
        async move {
            Some(ExtensionEntry::info(serde_json::json!({
                "offers": n,
                "resourceUrl": url,
            })))
        }
    }

    fn on_settle<'a>(
        &'a self,
        _ctx: &'a ExtSettleContext<'a>,
    ) -> impl Future<Output = Option<ExtensionEntry>> + Send + 'a {
        std::future::ready(Some(ExtensionEntry::info(
            serde_json::json!({ "receipt": true }),
        )))
    }
}

#[tokio::test]
async fn registered_extension_enriches_402() {
    let rs = ResourceServer::new(mock_ok())
        .with_scheme(eip155_wildcard(), MockScheme::authorization())
        .with_extension(OfferStub);
    let built = rs
        .create_payment_required_response(
            vec![sample_payload().accepted],
            PaymentRequiredBuildContext {
                resource: ResourceInfo::new("https://example.com/paid"),
                error: None,
                extensions: Extensions::new(),
                supported: SupportedResponse::default(),
                payment_payload: None,
            },
        )
        .await
        .unwrap();
    let info = built
        .extensions
        .get("offer-receipt")
        .unwrap()
        .as_info()
        .unwrap();
    assert_eq!(info["offers"], 1);
    assert_eq!(info["resourceUrl"], "https://example.com/paid");
}

#[tokio::test]
async fn registered_extension_attaches_settle_receipt() {
    let mock = mock_ok();
    let rs = ResourceServer::new(Arc::clone(&mock))
        .with_scheme(eip155_wildcard(), MockScheme::authorization())
        .with_extension(OfferStub);
    let payload = sample_payload().with_resource(ResourceInfo::new("https://example.com/paid"));
    let req = payload.accepted.clone();
    let settled = rs
        .settle_payment(&payload, &req, None, SettlePhase::AfterHandler, None, None)
        .await
        .unwrap();
    let info = match &settled {
        SettleResponse::Success { extensions, .. } => extensions
            .get("offer-receipt")
            .unwrap()
            .as_info()
            .unwrap()
            .clone(),
        SettleResponse::Failure { .. } | _ => panic!("expected success"),
    };
    assert_eq!(info["receipt"], true);
}

fn builder_code_entry(info: Value) -> ExtensionEntry {
    ExtensionEntry::info(info)
}

#[tokio::test]
async fn verify_rejects_forged_builder_code_a() {
    let mock = mock_ok();
    let rs = ResourceServer::new(Arc::clone(&mock))
        .with_scheme(eip155_wildcard(), MockScheme::authorization());
    let mut payload = sample_payload();
    payload.extensions.insert(
        "builder-code",
        builder_code_entry(serde_json::json!({"a": "forged_app"})),
    );
    let mut advertised = Extensions::new();
    advertised.insert(
        "builder-code",
        builder_code_entry(serde_json::json!({"a": "bc_app"})),
    );
    let req = payload.accepted.clone();
    let err = rs
        .verify_payment(&payload, &req, Some(&advertised))
        .await
        .unwrap_err();
    assert!(matches!(
        err,
        FacilitatorError::Verification(VerificationError::ExtensionEchoMismatch {
            ref extension_key
        }) if extension_key == "builder-code"
    ));
    assert_eq!(
        err.as_payment_problem().unwrap().reason(),
        ErrorReason::ExtensionEchoMismatch
    );
    assert_eq!(mock.verifies.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn verify_rejects_builder_code_a_without_declaration() {
    let rs =
        ResourceServer::new(mock_ok()).with_scheme(eip155_wildcard(), MockScheme::authorization());
    let mut payload = sample_payload();
    payload.extensions.insert(
        "builder-code",
        builder_code_entry(serde_json::json!({"a": "forged_app"})),
    );
    let req = payload.accepted.clone();
    let err = rs.verify_payment(&payload, &req, None).await.unwrap_err();
    assert!(matches!(
        err,
        FacilitatorError::Verification(VerificationError::ExtensionEchoMismatch { .. })
    ));
}

#[tokio::test]
async fn verify_rejects_builder_code_s_over_budget() {
    let rs =
        ResourceServer::new(mock_ok()).with_scheme(eip155_wildcard(), MockScheme::authorization());
    let mut padded = vec!["bc_server".to_owned()];
    padded.extend((0..10).map(|i| format!("bc_fake_{i}")));
    let mut payload = sample_payload();
    payload.extensions.insert(
        "builder-code",
        builder_code_entry(serde_json::json!({"a": "bc_app", "s": padded})),
    );
    let mut advertised = Extensions::new();
    advertised.insert(
        "builder-code",
        builder_code_entry(serde_json::json!({"a": "bc_app", "s": ["bc_server"]})),
    );
    let req = payload.accepted.clone();
    let err = rs
        .verify_payment(&payload, &req, Some(&advertised))
        .await
        .unwrap_err();
    assert!(matches!(
        err,
        FacilitatorError::Verification(VerificationError::ExtensionEchoMismatch { .. })
    ));
}

#[tokio::test]
async fn verify_accepts_matching_builder_code_a_and_superset_s() {
    let mock = mock_ok();
    let rs = ResourceServer::new(Arc::clone(&mock))
        .with_scheme(eip155_wildcard(), MockScheme::authorization());
    let mut payload = sample_payload();
    payload.extensions.insert(
        "builder-code",
        builder_code_entry(serde_json::json!({
            "a": "bc_app",
            "s": ["bc_server", "bc_client"]
        })),
    );
    let mut advertised = Extensions::new();
    advertised.insert(
        "builder-code",
        builder_code_entry(serde_json::json!({"a": "bc_app", "s": ["bc_server"]})),
    );
    let req = payload.accepted.clone();
    let out = rs
        .verify_payment(&payload, &req, Some(&advertised))
        .await
        .unwrap();
    assert!(out.response.is_valid());
    assert_eq!(mock.verifies.load(Ordering::SeqCst), 1);
}

#[test]
fn failure_path_prefers_successful_cancel() {
    let req = sample_payload().accepted;
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
            payer: Some("0xpayer".into()),
            transaction: "0xdeposit".into(),
            network: "eip155:8453".into(),
            amount: Some("1000".into()),
            extensions: Extensions::new(),
            extension_responses: Extensions::new(),
            extra: None,
        },
        req,
    );
    let cancel = SettleResponse::Failure {
        reason: ErrorReason::UnexpectedSettleError,
        message: Some("refund_failed".into()),
        payer: None,
        transaction: "should-clear".into(),
        network: "eip155:8453".into(),
        extensions: Extensions::new(),
        extension_responses: Extensions::new(),
        extra: None,
    };
    let got = build_failure_path_settlement_response(Some(&cancel), Some(&before), Some(&payload))
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
fn failure_path_none_when_nothing_available() {
    assert!(build_failure_path_settlement_response(None, None, None).is_none());
}
