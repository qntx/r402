#![allow(unused_crate_dependencies, reason = "sibling tests consume them")]
#![allow(
    clippy::tests_outside_test_module,
    reason = "integration test binaries put #[test] fns at file scope"
)]
#![allow(
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::items_after_statements,
    clippy::panic,
    clippy::unwrap_used,
    reason = "idiomatic test-code patterns"
)]

//! `PaymentWrapper` paymentFlow; facilitator Transport → tool `isError`.

use std::collections::HashMap;
use std::future::Future;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use compact_str::CompactString;
use r402_facilitator::Facilitator;
use r402_mcp::{
    McpPaymentPayload, PaymentWrapper, PaymentWrapperConfig, PaymentWrapperConfigError,
    attach_payment_to_params, extract_payment_required, extract_settle_response,
};
use r402_protocol::error::{ErrorReason, FacilitatorError, FacilitatorTransportKind};
use r402_protocol::network::{ChainId, ChainIdPattern};
use r402_protocol::payment::{
    Extensions, PaymentRequirements, ResourceInfo, SettleRequest, SettleResponse,
    SupportedPaymentKind, SupportedResponse, VerifyRequest, VerifyResponse,
};
use r402_server::{
    AfterVerifyDecision, BeforeOpDecision, FacilitatorSupportError, PaymentFlowConfig,
    PaymentFlowName, PaymentHookContext, ResourceServer, ResourceServerHooks,
    SDK_DEFAULT_ASSET_TRANSFER_METHOD, SchemeNetworkServer, SkipHandlerDirective,
    VerifiedPaymentCanceledContext, VerifyResultContext,
};
use rmcp::model::{CallToolRequestParams, CallToolResult, ContentBlock};
use serde_json::{Value, json};

#[derive(Clone, Copy)]
enum SupportedScript {
    Ok,
    Empty,
    Transport,
}

struct MockFacilitator {
    verifies: AtomicUsize,
    settles: AtomicUsize,
    verify_ok: bool,
    settle_ok: bool,
    fail_settle_from: Option<usize>,
    transport: bool,
    supported: SupportedScript,
}

impl MockFacilitator {
    fn new(verify_ok: bool, settle_ok: bool) -> Arc<Self> {
        Arc::new(Self {
            verifies: AtomicUsize::new(0),
            settles: AtomicUsize::new(0),
            verify_ok,
            settle_ok,
            fail_settle_from: None,
            transport: false,
            supported: SupportedScript::Ok,
        })
    }

    fn transport() -> Arc<Self> {
        Arc::new(Self {
            verifies: AtomicUsize::new(0),
            settles: AtomicUsize::new(0),
            verify_ok: true,
            settle_ok: true,
            fail_settle_from: None,
            transport: true,
            supported: SupportedScript::Ok,
        })
    }

    fn supported_script(script: SupportedScript) -> Arc<Self> {
        Arc::new(Self {
            verifies: AtomicUsize::new(0),
            settles: AtomicUsize::new(0),
            verify_ok: true,
            settle_ok: true,
            fail_settle_from: None,
            transport: false,
            supported: script,
        })
    }
}

impl Facilitator for MockFacilitator {
    fn verify(
        &self,
        _request: VerifyRequest,
    ) -> impl Future<Output = Result<VerifyResponse, FacilitatorError>> + Send {
        self.verifies.fetch_add(1, Ordering::SeqCst);
        std::future::ready(if self.transport {
            Err(FacilitatorError::transport(
                FacilitatorTransportKind::Timeout,
            ))
        } else if self.verify_ok {
            Ok(VerifyResponse::valid("0xpayer"))
        } else {
            Ok(VerifyResponse::invalid(
                None,
                ErrorReason::InvalidExactEvmPayloadAuthorizationValidAfter,
            ))
        })
    }

    fn settle(
        &self,
        _request: SettleRequest,
    ) -> impl Future<Output = Result<SettleResponse, FacilitatorError>> + Send {
        let n = self.settles.fetch_add(1, Ordering::SeqCst) + 1;
        std::future::ready(if self.transport {
            Err(FacilitatorError::transport(FacilitatorTransportKind::Io))
        } else {
            let fail = !self.settle_ok || self.fail_settle_from.is_some_and(|from| n >= from);
            Ok(if fail {
                mock_settle_failure()
            } else {
                mock_settle_success(n)
            })
        })
    }

    fn supported(
        &self,
    ) -> impl Future<Output = Result<SupportedResponse, FacilitatorError>> + Send {
        let out = match self.supported {
            SupportedScript::Ok => {
                let mut signers = HashMap::new();
                signers.insert("eip155:1".into(), vec!["0xfeePayer".into()]);
                Ok(SupportedResponse::new()
                    .with_kinds(vec![SupportedPaymentKind::new(2, "exact", "eip155:1")])
                    .with_signers(signers))
            }
            SupportedScript::Empty => Ok(SupportedResponse::new()),
            SupportedScript::Transport => Err(FacilitatorError::transport(
                FacilitatorTransportKind::Timeout,
            )),
        };
        std::future::ready(out)
    }
}

fn mock_settle_success(n: usize) -> SettleResponse {
    let (transaction, amount) = if n == 1 {
        ("0xtx", "1")
    } else {
        ("0xrefund", "0")
    };
    SettleResponse::Success {
        payer: Some("0xpayer".into()),
        transaction: transaction.into(),
        network: "eip155:1".into(),
        amount: Some(amount.into()),
        extensions: Extensions::new(),
        extension_responses: Extensions::new(),
        extra: None,
    }
}

fn mock_settle_failure() -> SettleResponse {
    SettleResponse::Failure {
        reason: ErrorReason::UnexpectedSettleError,
        message: Some("boom".into()),
        payer: None,
        transaction: "".into(),
        network: "eip155:1".into(),
        extensions: Extensions::new(),
        extension_responses: Extensions::new(),
        extra: None,
    }
}

fn accepts() -> Vec<PaymentRequirements> {
    vec![PaymentRequirements::new(
        "exact".into(),
        "eip155:1".parse().unwrap(),
        "1".into(),
        "0xa".into(),
        "0xb".into(),
        60,
    )]
}

fn sample_payload() -> McpPaymentPayload {
    let req = accepts().into_iter().next().expect("accepts fixture");
    McpPaymentPayload::new(req, json!({"sig": "0x"}))
}

fn payload_for(requirements: PaymentRequirements) -> McpPaymentPayload {
    McpPaymentPayload::new(requirements, json!({"sig": "0x"}))
}

fn with_payment_flow(mut requirements: PaymentRequirements, flow: &str) -> PaymentRequirements {
    requirements.extra = Some(json!({ "paymentFlow": flow }));
    requirements
}

struct FlowScheme {
    flows: HashMap<String, PaymentFlowConfig>,
    cancel: Option<PaymentRequirements>,
}

impl FlowScheme {
    fn with_config(config: PaymentFlowConfig, cancel: Option<PaymentRequirements>) -> Self {
        let mut flows = HashMap::new();
        flows.insert(SDK_DEFAULT_ASSET_TRANSFER_METHOD.into(), config);
        Self { flows, cancel }
    }

    fn authorization() -> Self {
        Self::with_config(PaymentFlowConfig::authorization_only(), None)
    }

    fn auth_and_upfront() -> Self {
        Self::with_config(PaymentFlowConfig::authorization_and_upfront(), None)
    }

    fn escrow(cancel: Option<PaymentRequirements>) -> Self {
        Self::with_config(
            PaymentFlowConfig::new(vec![PaymentFlowName::Escrow], PaymentFlowName::Escrow),
            cancel,
        )
    }
}

impl SchemeNetworkServer for FlowScheme {
    fn scheme(&self) -> &'static str {
        "exact"
    }

    fn default_asset_transfer_method(&self) -> &str {
        SDK_DEFAULT_ASSET_TRANSFER_METHOD
    }

    fn payment_flows(&self) -> &HashMap<String, PaymentFlowConfig> {
        &self.flows
    }

    fn settles_on_cancel(&self) -> bool {
        self.cancel.is_some()
    }

    fn settle_on_cancel<'a>(
        &'a self,
        _: &'a VerifiedPaymentCanceledContext,
    ) -> impl Future<Output = Option<PaymentRequirements>> + Send + 'a {
        std::future::ready(self.cancel.clone())
    }
}

struct FeePayerScheme(FlowScheme);

impl SchemeNetworkServer for FeePayerScheme {
    fn scheme(&self) -> &str {
        self.0.scheme()
    }

    fn default_asset_transfer_method(&self) -> &str {
        self.0.default_asset_transfer_method()
    }

    fn payment_flows(&self) -> &HashMap<String, PaymentFlowConfig> {
        self.0.payment_flows()
    }

    fn validate_facilitator_support(
        &self,
        network: &ChainId,
        kind: &SupportedPaymentKind,
        _facilitator_extensions: &[CompactString],
    ) -> Result<(), FacilitatorSupportError> {
        let has_fee_payer = kind
            .extra
            .as_ref()
            .and_then(|extra| extra.get("feePayer"))
            .and_then(Value::as_str)
            .is_some();
        if has_fee_payer {
            Ok(())
        } else {
            Err(FacilitatorSupportError::MissingFeePayer {
                scheme: self.scheme().into(),
                network: network.clone(),
            })
        }
    }
}

struct FlipEmptySupported {
    calls: AtomicUsize,
}

impl Facilitator for FlipEmptySupported {
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
        std::future::ready(Ok(mock_settle_success(1)))
    }

    fn supported(
        &self,
    ) -> impl Future<Output = Result<SupportedResponse, FacilitatorError>> + Send {
        let n = self.calls.fetch_add(1, Ordering::SeqCst);
        let out = if n == 0 {
            Ok(SupportedResponse::new()
                .with_kinds(vec![SupportedPaymentKind::new(2, "exact", "eip155:1")]))
        } else {
            Ok(SupportedResponse::new())
        };
        std::future::ready(out)
    }
}

fn eip155_wildcard() -> ChainIdPattern {
    ChainIdPattern::wildcard("eip155")
}

async fn wrapper_with(
    fac: Arc<MockFacilitator>,
    scheme: FlowScheme,
    accepts: Vec<PaymentRequirements>,
    hook: Option<impl ResourceServerHooks + 'static>,
) -> PaymentWrapper {
    let mut server = ResourceServer::new(fac);
    server.register_scheme(eip155_wildcard(), scheme);
    if let Some(hook) = hook {
        server.add_hook(hook);
    }
    let config = PaymentWrapperConfig::try_new(accepts, None).unwrap();
    PaymentWrapper::try_new(server, config).await.unwrap()
}

async fn wrapper(verify_ok: bool, settle_ok: bool) -> PaymentWrapper {
    let fac = MockFacilitator::new(verify_ok, settle_ok);
    let server =
        ResourceServer::new(fac).with_scheme(eip155_wildcard(), FlowScheme::authorization());
    let config = PaymentWrapperConfig::try_new(
        accepts(),
        Some(ResourceInfo::new("mcp://tool/demo").with_description("Demo")),
    )
    .unwrap();
    PaymentWrapper::try_new(server, config).await.unwrap()
}

#[tokio::test]
async fn missing_payment_returns_dual_format_402() {
    let result = wrapper(true, true)
        .await
        .invoke(CallToolRequestParams::new("demo"), |_| async {
            CallToolResult::success(vec![ContentBlock::text("should not run")])
        })
        .await;
    assert_eq!(result.is_error, Some(true));
    assert!(result.structured_content.is_some());
    let text = result
        .content
        .first()
        .and_then(ContentBlock::as_text)
        .unwrap();
    assert!(text.text.contains("Payment Required"));
}

#[tokio::test]
async fn happy_path_settles_and_attaches_meta() {
    let params = attach_payment_to_params(CallToolRequestParams::new("demo"), &sample_payload());
    let result = wrapper(true, true)
        .await
        .invoke(params, |_| async {
            CallToolResult::success(vec![ContentBlock::text("ok")])
        })
        .await;
    assert!(!result.is_error.unwrap_or(false));
    assert!(extract_settle_response(&result).unwrap().is_success());
}

#[tokio::test]
async fn tool_error_skips_settlement() {
    let fac = MockFacilitator::new(true, true);
    let settles = Arc::clone(&fac);
    let w = wrapper_with(
        fac,
        FlowScheme::authorization(),
        accepts(),
        None::<SkipHandlerHook>,
    )
    .await;
    let params = attach_payment_to_params(CallToolRequestParams::new("demo"), &sample_payload());
    let result = w
        .invoke(params, |_| async {
            CallToolResult::structured_error(json!({"err": true}))
        })
        .await;
    assert_eq!(result.is_error, Some(true));
    assert_eq!(settles.settles.load(Ordering::SeqCst), 0);
    assert_eq!(settles.verifies.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn verify_failure_is_tool_error_not_transport() {
    let params = attach_payment_to_params(CallToolRequestParams::new("demo"), &sample_payload());
    let result = wrapper(false, true)
        .await
        .invoke(params, |_| async {
            CallToolResult::success(vec![ContentBlock::text("nope")])
        })
        .await;
    assert_eq!(result.is_error, Some(true));
    assert!(result.structured_content.is_some());
}

#[tokio::test]
async fn settle_failure_uses_payment_required_format() {
    let params = attach_payment_to_params(CallToolRequestParams::new("demo"), &sample_payload());
    let result = wrapper(true, false)
        .await
        .invoke(params, |_| async {
            CallToolResult::success(vec![ContentBlock::text("ok")])
        })
        .await;
    assert_eq!(result.is_error, Some(true));
    let text = result
        .content
        .first()
        .and_then(ContentBlock::as_text)
        .unwrap();
    assert!(text.text.contains("Settlement failed"));
    assert!(extract_settle_response(&result).is_none());
}

#[tokio::test]
async fn facilitator_transport_verify_is_tool_is_error() {
    let w = wrapper_with(
        MockFacilitator::transport(),
        FlowScheme::authorization(),
        accepts(),
        None::<SkipHandlerHook>,
    )
    .await;
    let params = attach_payment_to_params(CallToolRequestParams::new("demo"), &sample_payload());
    let result = w
        .invoke(params, |_| async {
            CallToolResult::success(vec![ContentBlock::text("nope")])
        })
        .await;
    assert_eq!(
        result.is_error,
        Some(true),
        "Transport maps to tool isError, not HTTP 502"
    );
    assert!(
        extract_payment_required(&result).is_none(),
        "Transport must not be a 402 PaymentRequired the buyer would auto-pay"
    );
}

#[tokio::test]
async fn try_new_rejects_empty_supported_kinds() {
    let server = ResourceServer::new(MockFacilitator::supported_script(SupportedScript::Empty))
        .with_scheme(eip155_wildcard(), FlowScheme::authorization());
    let config = PaymentWrapperConfig::try_new(accepts(), None).unwrap();
    let err = PaymentWrapper::try_new(server, config).await.unwrap_err();
    assert!(matches!(
        err,
        PaymentWrapperConfigError::FacilitatorSupport(FacilitatorSupportError::KindMissing { .. })
    ));
}

#[tokio::test]
async fn try_new_rejects_supported_transport() {
    let server = ResourceServer::new(MockFacilitator::supported_script(
        SupportedScript::Transport,
    ))
    .with_scheme(eip155_wildcard(), FlowScheme::authorization());
    let config = PaymentWrapperConfig::try_new(accepts(), None).unwrap();
    let err = PaymentWrapper::try_new(server, config).await.unwrap_err();
    assert!(matches!(
        err,
        PaymentWrapperConfigError::SupportedTransport {
            kind: FacilitatorTransportKind::Timeout
        }
    ));
}

#[tokio::test]
async fn try_new_rejects_missing_fee_payer() {
    let server = ResourceServer::new(MockFacilitator::new(true, true)).with_scheme(
        eip155_wildcard(),
        FeePayerScheme(FlowScheme::authorization()),
    );
    let config = PaymentWrapperConfig::try_new(accepts(), None).unwrap();
    let err = PaymentWrapper::try_new(server, config).await.unwrap_err();
    assert!(matches!(
        err,
        PaymentWrapperConfigError::FacilitatorSupport(
            FacilitatorSupportError::MissingFeePayer { .. }
        )
    ));
}

#[tokio::test]
async fn invoke_empty_kinds_after_construct_is_tool_is_error() {
    let server = ResourceServer::new(Arc::new(FlipEmptySupported {
        calls: AtomicUsize::new(0),
    }))
    .with_scheme(eip155_wildcard(), FlowScheme::authorization());
    let config = PaymentWrapperConfig::try_new(accepts(), None).unwrap();
    let w = PaymentWrapper::try_new(server, config)
        .await
        .expect("first /supported has a kind");
    let result = w
        .invoke(CallToolRequestParams::new("demo"), |_| async {
            CallToolResult::success(vec![ContentBlock::text("should not run")])
        })
        .await;
    assert_eq!(result.is_error, Some(true));
    assert!(
        extract_payment_required(&result).is_none(),
        "empty /supported kinds must not emit PaymentRequired"
    );
}

#[tokio::test]
async fn facilitator_transport_settle_is_tool_is_error() {
    let fac = Arc::new(MockFacilitator {
        verifies: AtomicUsize::new(0),
        settles: AtomicUsize::new(0),
        verify_ok: true,
        settle_ok: true,
        fail_settle_from: None,
        transport: true,
        supported: SupportedScript::Ok,
    });
    // Authorization verify hits Transport first; use skip-verify hook so settle is the transport.
    struct SkipVerify;
    impl ResourceServerHooks for SkipVerify {
        fn before_verify<'a>(
            &'a self,
            _: &'a PaymentHookContext,
        ) -> impl Future<Output = BeforeOpDecision<VerifyResponse>> + Send + 'a {
            std::future::ready(BeforeOpDecision::Skip {
                result: VerifyResponse::valid("0xlocal"),
            })
        }
    }
    let w = wrapper_with(
        fac,
        FlowScheme::authorization(),
        accepts(),
        Some(SkipVerify),
    )
    .await;
    let params = attach_payment_to_params(CallToolRequestParams::new("demo"), &sample_payload());
    let result = w
        .invoke(params, |_| async {
            CallToolResult::success(vec![ContentBlock::text("ok")])
        })
        .await;
    assert_eq!(
        result.is_error,
        Some(true),
        "settle Transport is tool isError"
    );
    assert!(
        extract_payment_required(&result).is_none(),
        "Transport must not be a 402 PaymentRequired the buyer would auto-pay"
    );
}

struct SkipHandlerHook;

impl ResourceServerHooks for SkipHandlerHook {
    fn after_verify<'a>(
        &'a self,
        _ctx: &'a VerifyResultContext,
    ) -> impl Future<Output = AfterVerifyDecision> + Send + 'a {
        std::future::ready(AfterVerifyDecision::SkipHandler {
            response: SkipHandlerDirective::empty()
                .with_body(json!({"skipped": true, "source": "hook"})),
        })
    }
}

async fn never_run_tool(_params: CallToolRequestParams) -> CallToolResult {
    CallToolResult::success(vec![ContentBlock::text("should not run")])
}

#[tokio::test]
async fn skip_handler_settles_with_directive_body() {
    let fac = MockFacilitator::new(true, true);
    let settles = Arc::clone(&fac);
    let w = wrapper_with(
        fac,
        FlowScheme::authorization(),
        accepts(),
        Some(SkipHandlerHook),
    )
    .await;
    let params = attach_payment_to_params(CallToolRequestParams::new("demo"), &sample_payload());
    let result = w.invoke(params, never_run_tool).await;
    assert_eq!(settles.settles.load(Ordering::SeqCst), 1);
    assert!(!result.is_error.unwrap_or(false));
    assert_eq!(
        result
            .structured_content
            .as_ref()
            .and_then(|v| v.get("skipped")),
        Some(&json!(true))
    );
}

#[test]
fn empty_accepts_rejected() {
    let err = PaymentWrapperConfig::try_new(vec![], None).unwrap_err();
    assert!(matches!(err, PaymentWrapperConfigError::EmptyAccepts));
}

#[tokio::test]
async fn try_new_rejects_missing_scheme() {
    let server = ResourceServer::new(MockFacilitator::new(true, true));
    let config = PaymentWrapperConfig::try_new(accepts(), None).unwrap();
    let err = PaymentWrapper::try_new(server, config).await.unwrap_err();
    assert!(matches!(
        err,
        PaymentWrapperConfigError::MissingScheme { .. }
    ));
}

#[tokio::test]
async fn try_new_rejects_escrow_without_settle_on_cancel() {
    let server = ResourceServer::new(MockFacilitator::new(true, true))
        .with_scheme(eip155_wildcard(), FlowScheme::escrow(None));
    let config = PaymentWrapperConfig::try_new(accepts(), None).unwrap();
    let err = PaymentWrapper::try_new(server, config).await.unwrap_err();
    assert!(matches!(
        err,
        PaymentWrapperConfigError::MissingSettleOnCancel { .. }
    ));
}

#[tokio::test]
async fn upfront_settles_before_handler_and_echoes() {
    let fac = MockFacilitator::new(true, true);
    let settles = Arc::clone(&fac);
    let requirements = with_payment_flow(accepts().into_iter().next().unwrap(), "upfront");
    let w = wrapper_with(
        fac,
        FlowScheme::auth_and_upfront(),
        vec![requirements.clone()],
        None::<SkipHandlerHook>,
    )
    .await;
    let params = attach_payment_to_params(
        CallToolRequestParams::new("demo"),
        &payload_for(requirements),
    );
    let result = w
        .invoke(params, |_| async {
            CallToolResult::success(vec![ContentBlock::text("ok")])
        })
        .await;
    assert!(!result.is_error.unwrap_or(false));
    assert_eq!(settles.verifies.load(Ordering::SeqCst), 0);
    assert_eq!(settles.settles.load(Ordering::SeqCst), 1);
    assert!(extract_settle_response(&result).unwrap().is_success());
}

#[tokio::test]
async fn escrow_success_settles_before_and_after() {
    let fac = MockFacilitator::new(true, true);
    let settles = Arc::clone(&fac);
    let requirements = with_payment_flow(accepts().into_iter().next().unwrap(), "escrow");
    let w = wrapper_with(
        fac,
        FlowScheme::escrow(Some(requirements.clone())),
        vec![requirements.clone()],
        None::<SkipHandlerHook>,
    )
    .await;
    let params = attach_payment_to_params(
        CallToolRequestParams::new("demo"),
        &payload_for(requirements),
    );
    let result = w
        .invoke(params, |_| async {
            CallToolResult::success(vec![ContentBlock::text("ok")])
        })
        .await;
    assert!(!result.is_error.unwrap_or(false));
    assert_eq!(settles.verifies.load(Ordering::SeqCst), 0);
    assert_eq!(settles.settles.load(Ordering::SeqCst), 2);
    assert!(extract_settle_response(&result).unwrap().is_success());
}

#[tokio::test]
async fn escrow_tool_error_prefers_cancel_receipt() {
    let fac = MockFacilitator::new(true, true);
    let settles = Arc::clone(&fac);
    let requirements = with_payment_flow(accepts().into_iter().next().unwrap(), "escrow");
    let w = wrapper_with(
        fac,
        FlowScheme::escrow(Some(requirements.clone())),
        vec![requirements.clone()],
        None::<SkipHandlerHook>,
    )
    .await;
    let params = attach_payment_to_params(
        CallToolRequestParams::new("demo"),
        &payload_for(requirements),
    );
    let result = w
        .invoke(params, |_| async {
            CallToolResult::structured_error(json!({"err": true}))
        })
        .await;
    assert_eq!(result.is_error, Some(true));
    assert_eq!(settles.settles.load(Ordering::SeqCst), 2);
    match extract_settle_response(&result) {
        Some(SettleResponse::Success {
            ref transaction, ..
        }) => {
            assert_eq!(transaction.as_str(), "0xrefund");
        }
        other => panic!("expected cancel receipt, got {other:?}"),
    }
}

#[tokio::test]
async fn escrow_failed_cancel_includes_deposit_recovery() {
    let fac = Arc::new(MockFacilitator {
        verifies: AtomicUsize::new(0),
        settles: AtomicUsize::new(0),
        verify_ok: true,
        settle_ok: true,
        fail_settle_from: Some(2),
        transport: false,
        supported: SupportedScript::Ok,
    });
    let requirements = with_payment_flow(accepts().into_iter().next().unwrap(), "escrow");
    let w = wrapper_with(
        Arc::clone(&fac),
        FlowScheme::escrow(Some(requirements.clone())),
        vec![requirements.clone()],
        None::<SkipHandlerHook>,
    )
    .await;
    let params = attach_payment_to_params(
        CallToolRequestParams::new("demo"),
        &McpPaymentPayload::new(
            requirements,
            json!({"sig": "0x", "channelId": "channel-123"}),
        ),
    );
    let result = w
        .invoke(params, |_| async {
            CallToolResult::structured_error(json!({}))
        })
        .await;
    assert_eq!(result.is_error, Some(true));
    match extract_settle_response(&result) {
        Some(SettleResponse::Failure {
            ref transaction,
            ref extra,
            ..
        }) => {
            assert!(transaction.is_empty());
            let extra = extra.as_ref().and_then(Value::as_object).unwrap();
            assert_eq!(extra.get("depositTransaction"), Some(&json!("0xtx")));
            assert_eq!(extra.get("depositAmount"), Some(&json!("1")));
            assert_eq!(extra.get("channelId"), Some(&json!("channel-123")));
        }
        other => panic!("expected failed cancel receipt, got {other:?}"),
    }
}

struct SkipVerifyThenHandler;

impl ResourceServerHooks for SkipVerifyThenHandler {
    fn before_verify<'a>(
        &'a self,
        _: &'a PaymentHookContext,
    ) -> impl Future<Output = BeforeOpDecision<VerifyResponse>> + Send + 'a {
        std::future::ready(BeforeOpDecision::Skip {
            result: VerifyResponse::valid("0xlocal"),
        })
    }

    fn after_verify<'a>(
        &'a self,
        _: &VerifyResultContext,
    ) -> impl Future<Output = AfterVerifyDecision> + Send + 'a {
        std::future::ready(AfterVerifyDecision::SkipHandler {
            response: SkipHandlerDirective::empty().with_body(json!({"skipped": true})),
        })
    }
}

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

#[tokio::test]
async fn skip_handler_upfront_does_not_settle_or_cancel() {
    let fac = MockFacilitator::new(true, true);
    let cancels = Arc::new(AtomicUsize::new(0));
    let settles = Arc::clone(&fac);
    let requirements = with_payment_flow(accepts().into_iter().next().unwrap(), "upfront");
    let mut server = ResourceServer::new(Arc::clone(&fac));
    server.register_scheme(eip155_wildcard(), FlowScheme::auth_and_upfront());
    server.add_hook(SkipVerifyThenHandler);
    server.add_hook(CancelCountHook(Arc::clone(&cancels)));
    let config = PaymentWrapperConfig::try_new(vec![requirements.clone()], None).unwrap();
    let w = PaymentWrapper::try_new(server, config).await.unwrap();
    let params = attach_payment_to_params(
        CallToolRequestParams::new("demo"),
        &payload_for(requirements),
    );
    let result = w.invoke(params, never_run_tool).await;
    assert!(!result.is_error.unwrap_or(false));
    assert_eq!(settles.settles.load(Ordering::SeqCst), 0);
    assert_eq!(settles.verifies.load(Ordering::SeqCst), 0);
    assert_eq!(cancels.load(Ordering::SeqCst), 0);
    assert!(extract_settle_response(&result).is_some());
}

#[tokio::test]
async fn invoke_matches_pipeline_accepts_not_raw_config() {
    let fac = MockFacilitator::new(true, true);
    let advertised = PaymentRequirements::new(
        "exact".into(),
        "eip155:1".parse().unwrap(),
        "1".into(),
        "0xa".into(),
        "0xb".into(),
        60,
    )
    .with_extra(json!({"feePayer": "payer"}));
    let server =
        ResourceServer::new(fac).with_scheme(eip155_wildcard(), FlowScheme::authorization());
    let config = PaymentWrapperConfig::try_new(vec![advertised], None).unwrap();
    let w = PaymentWrapper::try_new(server, config).await.unwrap();

    let omitted = sample_payload();
    let omitted_result = w
        .invoke(
            attach_payment_to_params(CallToolRequestParams::new("demo"), &omitted),
            |_| async { CallToolResult::success(vec![ContentBlock::text("should not run")]) },
        )
        .await;
    assert_eq!(omitted_result.is_error, Some(true));
    let pr = extract_payment_required(&omitted_result).unwrap();
    assert_eq!(
        pr.accepts
            .first()
            .and_then(|accept| accept.extra.as_ref())
            .and_then(|extra| extra.get("feePayer"))
            .and_then(Value::as_str),
        Some("payer"),
    );

    let mut echoed = sample_payload();
    echoed.accepted.extra = pr.accepts.first().and_then(|accept| accept.extra.clone());
    let echoed_result = w
        .invoke(
            attach_payment_to_params(CallToolRequestParams::new("demo"), &echoed),
            |_| async { CallToolResult::success(vec![ContentBlock::text("ok")]) },
        )
        .await;
    assert!(!echoed_result.is_error.unwrap_or(false));
}
