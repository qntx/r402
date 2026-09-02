//! Server-side MCP payment wrapper.
//!
//! Ports Go `go/mcp/server.go` `PaymentWrapper.Wrap` control flow:
//! extract meta payment → match accepts → verify → hooks → handler → settle → meta.
//!
//! Verify/settle failures return **tool-level** dual-format payment-required
//! results (not transport errors), matching the official Go SDK.

use std::future::Future;
use std::sync::Arc;

use r402_core::resource_server::{
    CompletedSettlement, PaymentFlowName, PaymentRequiredBuildContext, ResourceServer, SettlePhase,
    SkipHandlerDirective, resolve_payment_flow_phases,
};
use r402_core::wire::{
    Extensions, PaymentRequired, PaymentRequirements, ResourceInfo, SettleResponse,
};
use r402_core::{CancelReason, PaymentFlowError, build_failure_path_settlement_response};
use rmcp::model::{CallToolRequestParams, CallToolResult, ContentBlock};
use serde_json::Value;

use crate::MCP_TOOL_URL_PREFIX;
use crate::encode::{
    McpPaymentPayload, attach_settle_response, create_tool_resource_url,
    extract_payment_from_params, payment_required_tool_result, settlement_failed_tool_result,
};

/// Error constructing a [`PaymentWrapper`].
#[derive(Debug, Clone, thiserror::Error)]
pub enum PaymentWrapperConfigError {
    /// `accepts` must be non-empty (Go panics; we return an error).
    #[error("PaymentWrapperConfig.accepts must have at least one payment requirement")]
    EmptyAccepts,
    /// No [`r402_core::SchemeNetworkServer`] registered for this accept.
    #[error("missing scheme {scheme} on {network}")]
    MissingScheme {
        /// Wire scheme name.
        scheme: String,
        /// Accept CAIP-2 network.
        network: String,
    },
    /// [`r402_core::resolve_payment_flow`] failed for an accept.
    #[error(transparent)]
    UnresolvedPaymentFlow(#[from] PaymentFlowError),
    /// Escrow accept whose scheme does not implement `settle_on_cancel`.
    #[error("escrow scheme {scheme} is missing settle_on_cancel")]
    MissingSettleOnCancel {
        /// Wire scheme name.
        scheme: String,
    },
}

/// Builds a successful tool result from a [`SkipHandlerDirective`].
///
/// JSON bodies become `structuredContent`; string bodies become text content;
/// empty directives yield a `null` structured success (mirrors HTTP JSON body).
fn settle_failure_reason(settle: &SettleResponse) -> String {
    match settle {
        SettleResponse::Failure {
            reason, message, ..
        } => message
            .as_ref()
            .map_or_else(|| reason.to_string(), ToString::to_string),
        _ => "Settlement failed".into(),
    }
}

fn internal_server_error_result(settle: Option<&SettleResponse>) -> CallToolResult {
    let result = CallToolResult::error(vec![ContentBlock::text("Internal Server Error")]);
    match settle {
        Some(settle) => attach_settle_response(result, settle),
        None => result,
    }
}

fn skip_handler_tool_result(directive: &SkipHandlerDirective) -> CallToolResult {
    match &directive.body {
        Some(Value::String(s)) => CallToolResult::success(vec![ContentBlock::text(s.clone())]),
        Some(value) => CallToolResult::structured(value.clone()),
        None => CallToolResult::structured(Value::Null),
    }
}

/// Server-side lifecycle hooks (Go `PaymentWrapperHooks`).
#[derive(Clone, Default)]
pub struct PaymentWrapperHooks {
    /// After verify, before tool. Return `false` to abort.
    pub on_before_execution: Option<Arc<dyn Fn(ServerHookContext) -> bool + Send + Sync>>,
    /// After tool success, before settle (non-fatal).
    pub on_after_execution: Option<Arc<dyn Fn(AfterExecutionContext) + Send + Sync>>,
    /// After successful settle (non-fatal).
    pub on_after_settlement: Option<Arc<dyn Fn(SettlementContext) + Send + Sync>>,
}

impl std::fmt::Debug for PaymentWrapperHooks {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PaymentWrapperHooks")
            .field("on_before_execution", &self.on_before_execution.is_some())
            .field("on_after_execution", &self.on_after_execution.is_some())
            .field("on_after_settlement", &self.on_after_settlement.is_some())
            .finish()
    }
}

/// Go `ServerHookContext`.
#[derive(Debug, Clone)]
pub struct ServerHookContext {
    /// Tool name.
    pub tool_name: String,
    /// Tool arguments (JSON object keys).
    pub arguments: serde_json::Map<String, Value>,
    /// Matched requirements.
    pub payment_requirements: PaymentRequirements,
    /// Client payment payload.
    pub payment_payload: McpPaymentPayload,
}

/// Go `AfterExecutionContext`.
#[derive(Debug, Clone)]
pub struct AfterExecutionContext {
    /// Shared server context.
    pub server: ServerHookContext,
    /// Tool result before settle.
    pub result: CallToolResult,
}

/// Go `SettlementContext`.
#[derive(Debug, Clone)]
pub struct SettlementContext {
    /// Shared server context.
    pub server: ServerHookContext,
    /// Successful settlement response.
    pub settlement: SettleResponse,
}

/// Go `PaymentWrapperConfig`.
#[derive(Debug, Clone)]
pub struct PaymentWrapperConfig {
    /// Advertised payment options.
    pub accepts: Vec<PaymentRequirements>,
    /// Resource metadata (defaulted if None at use sites).
    pub resource: Option<ResourceInfo>,
    /// Optional lifecycle hooks.
    pub hooks: PaymentWrapperHooks,
    /// Extensions included on payment-required responses.
    pub extensions: Extensions,
}

impl PaymentWrapperConfig {
    /// Builds config; returns error if `accepts` is empty.
    ///
    /// # Errors
    ///
    /// [`PaymentWrapperConfigError::EmptyAccepts`] when `accepts` is empty.
    pub fn try_new(
        accepts: Vec<PaymentRequirements>,
        resource: Option<ResourceInfo>,
    ) -> Result<Self, PaymentWrapperConfigError> {
        if accepts.is_empty() {
            return Err(PaymentWrapperConfigError::EmptyAccepts);
        }
        Ok(Self {
            accepts,
            resource,
            hooks: PaymentWrapperHooks::default(),
            extensions: Extensions::new(),
        })
    }

    /// Attaches hooks.
    #[must_use]
    pub fn with_hooks(mut self, hooks: PaymentWrapperHooks) -> Self {
        self.hooks = hooks;
        self
    }

    /// Attaches extensions for payment-required responses.
    #[must_use]
    pub fn with_extensions(mut self, extensions: Extensions) -> Self {
        self.extensions = extensions;
        self
    }
}

/// Go `PaymentWrapper`.
#[derive(Debug, Clone)]
pub struct PaymentWrapper {
    server: ResourceServer,
    config: PaymentWrapperConfig,
}

impl PaymentWrapper {
    /// Go `NewPaymentWrapper`.
    ///
    /// Requires a registered scheme and a resolvable payment flow per accept.
    /// Escrow additionally requires `settle_on_cancel`.
    ///
    /// # Errors
    ///
    /// Empty accepts, missing scheme, unresolved flow, or escrow without
    /// `settle_on_cancel`.
    pub async fn try_new(
        server: ResourceServer,
        config: PaymentWrapperConfig,
    ) -> Result<Self, PaymentWrapperConfigError> {
        if config.accepts.is_empty() {
            return Err(PaymentWrapperConfigError::EmptyAccepts);
        }
        for requirement in &config.accepts {
            if server
                .registered_scheme(requirement.scheme.as_str(), &requirement.network)
                .is_none()
            {
                return Err(PaymentWrapperConfigError::MissingScheme {
                    scheme: requirement.scheme.to_string(),
                    network: requirement.network.to_string(),
                });
            }
            let flow = server.get_payment_flow(requirement)?;
            if flow == PaymentFlowName::Escrow && !server.has_settle_on_cancel(requirement).await {
                return Err(PaymentWrapperConfigError::MissingSettleOnCancel {
                    scheme: requirement.scheme.to_string(),
                });
            }
        }
        Ok(Self { server, config })
    }

    /// Go `buildToolResourceInfo`.
    fn build_tool_resource_info(&self, tool_name: &str) -> ResourceInfo {
        if let Some(info) = &self.config.resource {
            let mut info = info.clone();
            if info.url.is_empty() && !tool_name.is_empty() {
                info.url = create_tool_resource_url(tool_name, None).into();
            }
            if info.description.as_deref().is_none_or(str::is_empty) && !tool_name.is_empty() {
                info.description = Some(format!("Tool: {tool_name}").into());
            }
            if info.mime_type.as_deref().is_none_or(str::is_empty) {
                info.mime_type = Some("application/json".into());
            }
            return info;
        }
        if tool_name.is_empty() {
            ResourceInfo::new(format!("{MCP_TOOL_URL_PREFIX}unknown"))
                .with_description("Unknown tool")
                .with_mime_type("application/json")
        } else {
            ResourceInfo::new(create_tool_resource_url(tool_name, None))
                .with_description(format!("Tool: {tool_name}"))
                .with_mime_type("application/json")
        }
    }

    /// Go `paymentRequiredResult`.
    pub async fn payment_required_result(&self, error_msg: impl Into<String>) -> CallToolResult {
        self.payment_required_result_for("", error_msg, None).await
    }

    async fn payment_required_result_for(
        &self,
        tool_name: &str,
        error_msg: impl Into<String>,
        payload: Option<&McpPaymentPayload>,
    ) -> CallToolResult {
        match self
            .create_payment_required(tool_name, error_msg.into(), payload)
            .await
        {
            Ok(required) => payment_required_tool_result(&required),
            Err(err) => CallToolResult::structured_error(serde_json::json!({
                "error": err.to_string(),
            })),
        }
    }

    /// Go `settlementFailedResult` (R5: same dual format as payment-required).
    pub async fn settlement_failed_result(&self, error_msg: impl Into<String>) -> CallToolResult {
        self.settlement_failed_result_for("", error_msg).await
    }

    async fn settlement_failed_result_for(
        &self,
        tool_name: &str,
        error_msg: impl Into<String>,
    ) -> CallToolResult {
        match self
            .create_payment_required(tool_name, error_msg.into(), None)
            .await
        {
            Ok(required) => settlement_failed_tool_result(&required),
            Err(err) => CallToolResult::structured_error(serde_json::json!({
                "error": err.to_string(),
            })),
        }
    }

    async fn create_payment_required(
        &self,
        tool_name: &str,
        error_msg: String,
        payload: Option<&McpPaymentPayload>,
    ) -> Result<PaymentRequired, r402_core::FacilitatorError> {
        let supported = self
            .server
            .facilitator()
            .supported()
            .await
            .unwrap_or_default();
        self.server
            .create_payment_required_response(
                self.config.accepts.clone(),
                PaymentRequiredBuildContext {
                    resource: self.build_tool_resource_info(tool_name),
                    error: Some(error_msg.into()),
                    extensions: self.config.extensions.clone(),
                    supported,
                    payment_payload: payload.cloned(),
                },
            )
            .await
    }

    /// Official HTTP/MCP `processSettlement`: after-handler no-ops when
    /// `!settleAfterHandler` (echo before-handler, else empty success).
    async fn process_settlement(
        &self,
        payload: &McpPaymentPayload,
        requirements: &PaymentRequirements,
        phase: SettlePhase,
        before_handler: Option<&CompletedSettlement>,
    ) -> Result<SettleResponse, r402_core::FacilitatorError> {
        let flow = self.server.get_payment_flow(requirements).map_err(|err| {
            r402_core::FacilitatorError::Verification(r402_core::VerificationError::InvalidFormat(
                err.to_string(),
            ))
        })?;
        let phases = resolve_payment_flow_phases(flow);
        if phase != SettlePhase::BeforeHandler && !phases.settle_after_handler {
            if let Some(before) = before_handler {
                return Ok(before.result.clone());
            }
            return Ok(SettleResponse::Success {
                payer: String::new().into(),
                transaction: String::new().into(),
                network: requirements.network.to_string().into(),
                amount: None,
                extensions: Extensions::new(),
                extra: None,
            });
        }
        self.server
            .settle_payment(payload, requirements, None, phase)
            .await
    }

    /// Full pipeline (Go `Wrap` body). Prefer [`Self::wrap`] for rmcp registration.
    #[allow(
        clippy::too_many_lines,
        clippy::cognitive_complexity,
        reason = "Go Wrap control flow is one sequential pipeline"
    )]
    pub async fn invoke<H, Fut>(&self, params: CallToolRequestParams, handler: H) -> CallToolResult
    where
        H: FnOnce(CallToolRequestParams) -> Fut,
        Fut: Future<Output = CallToolResult>,
    {
        let tool_name = params.name.to_string();
        let payload = extract_payment_from_params(&params);
        let challenge = match self
            .create_payment_required(&tool_name, "Payment Required".into(), payload.as_ref())
            .await
        {
            Ok(pr) => pr,
            Err(err) => {
                return CallToolResult::structured_error(serde_json::json!({
                    "error": err.to_string(),
                }));
            }
        };

        let Some(payload) = payload else {
            return payment_required_tool_result(&challenge);
        };

        let Some(requirements) = self
            .server
            .find_matching_requirements(&challenge.accepts, &payload)
            .cloned()
        else {
            let mut unmatched = challenge.clone();
            unmatched.error = Some("No matching payment requirements found".into());
            return payment_required_tool_result(&unmatched);
        };

        let Ok(flow) = self.server.get_payment_flow(&requirements) else {
            return internal_server_error_result(None);
        };
        let phases = resolve_payment_flow_phases(flow);

        let verify_out = match self.server.verify_payment(&payload, &requirements).await {
            Ok(out) if out.response.is_valid() => out,
            Ok(out) => {
                let reason = match out.response {
                    r402_core::wire::VerifyResponse::Invalid {
                        reason, message, ..
                    } => message.map_or_else(|| reason.to_string(), |m| m.to_string()),
                    // Valid handled above; `_` for non_exhaustive future variants.
                    _ => "Payment verification failed".into(),
                };
                return self
                    .payment_required_result_for(
                        &tool_name,
                        format!("Payment verification failed: {reason}"),
                        Some(&payload),
                    )
                    .await;
            }
            Err(err) => {
                return self
                    .payment_required_result_for(
                        &tool_name,
                        format!("Payment verification error: {err}"),
                        Some(&payload),
                    )
                    .await;
            }
        };

        let arguments = params.arguments.clone().unwrap_or_default();
        let hook_ctx = ServerHookContext {
            tool_name: tool_name.clone(),
            arguments,
            payment_requirements: requirements.clone(),
            payment_payload: payload.clone(),
        };

        // SkipHandler: processSettlement(AfterHandler) no-op when
        // !settleAfterHandler. Do not create a cancel dispatcher.
        if let Some(ref directive) = verify_out.skip_handler {
            let skip_result = skip_handler_tool_result(directive);
            return self
                .settle_payment_result(
                    &hook_ctx,
                    &payload,
                    &requirements,
                    skip_result,
                    None,
                    &tool_name,
                )
                .await;
        }

        let before_handler = if phases.settle_before_handler {
            match self
                .process_settlement(&payload, &requirements, SettlePhase::BeforeHandler, None)
                .await
            {
                Ok(result) if result.is_success() => Some(CompletedSettlement::new(
                    SettlePhase::BeforeHandler,
                    flow,
                    result,
                    requirements.clone(),
                )),
                Ok(result) => {
                    return self
                        .settlement_failed_result_for(
                            &tool_name,
                            format!("Settlement failed: {}", settle_failure_reason(&result)),
                        )
                        .await;
                }
                Err(_) => {
                    return self
                        .settlement_failed_result_for(&tool_name, "Settlement failed")
                        .await;
                }
            }
        } else {
            None
        };

        let mut cancel = self
            .server
            .cancellation_guard(payload.clone(), requirements.clone());
        if let Some(completed) = before_handler.as_ref() {
            cancel = cancel
                .with_settled_phases([SettlePhase::BeforeHandler])
                .with_before_handler(completed.clone());
        }

        if let Some(ref before) = self.config.hooks.on_before_execution
            && !before(hook_ctx.clone())
        {
            return self
                .payment_required_result_for(
                    &tool_name,
                    "Execution aborted by OnBeforeExecution hook",
                    None,
                )
                .await;
        }

        let result = handler(params).await;

        if let Some(ref after) = self.config.hooks.on_after_execution {
            after(AfterExecutionContext {
                server: hook_ctx.clone(),
                result: result.clone(),
            });
        }

        if result.is_error.unwrap_or(false) {
            let cancel_settlement = cancel
                .cancel(
                    CancelReason::HandlerFailed,
                    Some("tool returned isError"),
                    None,
                )
                .await;
            if let Some(receipt) = build_failure_path_settlement_response(
                cancel_settlement.as_ref(),
                before_handler.as_ref(),
                Some(&payload),
            ) {
                return attach_settle_response(result, &receipt);
            }
            return result;
        }

        self.settle_payment_result(
            &hook_ctx,
            &payload,
            &requirements,
            result,
            before_handler.as_ref(),
            &tool_name,
        )
        .await
    }

    /// Go `settlePaymentResult` via `processSettlement(AfterHandler)`.
    async fn settle_payment_result(
        &self,
        hook_ctx: &ServerHookContext,
        payload: &McpPaymentPayload,
        requirements: &PaymentRequirements,
        result: CallToolResult,
        before_handler: Option<&CompletedSettlement>,
        tool_name: &str,
    ) -> CallToolResult {
        let settle = match self
            .process_settlement(
                payload,
                requirements,
                SettlePhase::AfterHandler,
                before_handler,
            )
            .await
        {
            Ok(s) if s.is_success() => s,
            Ok(s) => {
                return self
                    .settlement_failed_result_for(
                        tool_name,
                        format!("Settlement failed: {}", settle_failure_reason(&s)),
                    )
                    .await;
            }
            Err(_) => {
                return self
                    .settlement_failed_result_for(tool_name, "Settlement failed")
                    .await;
            }
        };

        if let Some(ref after_settle) = self.config.hooks.on_after_settlement {
            after_settle(SettlementContext {
                server: hook_ctx.clone(),
                settlement: settle.clone(),
            });
        }

        attach_settle_response(result, &settle)
    }

    /// Go `Wrap`: returns a cloneable async handler for rmcp tool routers.
    pub fn wrap<H, Fut>(
        &self,
        handler: H,
    ) -> impl Fn(
        CallToolRequestParams,
    ) -> std::pin::Pin<Box<dyn Future<Output = CallToolResult> + Send>>
    + Clone
    + Send
    + Sync
    + 'static
    where
        H: Fn(CallToolRequestParams) -> Fut + Clone + Send + Sync + 'static,
        Fut: Future<Output = CallToolResult> + Send + 'static,
    {
        let this = self.clone();
        move |params| {
            let this = this.clone();
            let handler = handler.clone();
            Box::pin(async move { this.invoke(params, handler).await })
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::future::Future;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use r402_core::FacilitatorError;
    use r402_core::chain::ChainIdPattern;
    use r402_core::facilitator::Facilitator;
    use r402_core::resource_server::{
        AfterVerifyDecision, BeforeOpDecision, PaymentFlowConfig, PaymentHookContext,
        ResourceServerHooks, SDK_DEFAULT_ASSET_TRANSFER_METHOD, SchemeNetworkServer,
        VerifiedPaymentCanceledContext, VerifyResultContext,
    };
    use r402_core::wire::{
        PaymentRequirements, ResourceInfo, SettleRequest, SettleResponse, SupportedResponse,
        VerifyRequest, VerifyResponse,
    };
    use rmcp::model::ContentBlock;
    use serde_json::json;

    use super::*;
    use crate::encode::{
        McpPaymentPayload, attach_payment_to_params, extract_payment_required,
        extract_settle_response,
    };

    struct MockFacilitator {
        verifies: AtomicUsize,
        settles: AtomicUsize,
        verify_ok: bool,
        settle_ok: bool,
        fail_settle_from: Option<usize>,
    }

    impl MockFacilitator {
        fn new(verify_ok: bool, settle_ok: bool) -> Arc<Self> {
            Arc::new(Self {
                verifies: AtomicUsize::new(0),
                settles: AtomicUsize::new(0),
                verify_ok,
                settle_ok,
                fail_settle_from: None,
            })
        }
    }

    impl Facilitator for MockFacilitator {
        fn verify(
            &self,
            _request: VerifyRequest,
        ) -> impl Future<Output = Result<VerifyResponse, FacilitatorError>> + Send {
            self.verifies.fetch_add(1, Ordering::SeqCst);
            std::future::ready(if self.verify_ok {
                Ok(VerifyResponse::valid("0xpayer"))
            } else {
                Ok(VerifyResponse::invalid(
                    None,
                    r402_core::ErrorReason::InvalidExactEvmPayloadAuthorizationValidAfter,
                ))
            })
        }

        fn settle(
            &self,
            _request: SettleRequest,
        ) -> impl Future<Output = Result<SettleResponse, FacilitatorError>> + Send {
            let n = self.settles.fetch_add(1, Ordering::SeqCst) + 1;
            let fail = !self.settle_ok || self.fail_settle_from.is_some_and(|from| n >= from);
            std::future::ready(Ok(if fail {
                mock_settle_failure()
            } else {
                mock_settle_success(n)
            }))
        }

        fn supported(
            &self,
        ) -> impl Future<Output = Result<SupportedResponse, FacilitatorError>> + Send {
            std::future::ready(Ok(SupportedResponse::default()))
        }
    }

    fn mock_settle_success(n: usize) -> SettleResponse {
        let (transaction, amount) = if n == 1 {
            ("0xtx", "1")
        } else {
            ("0xrefund", "0")
        };
        SettleResponse::Success {
            payer: "0xpayer".into(),
            transaction: transaction.into(),
            network: "eip155:1".into(),
            amount: Some(amount.into()),
            extensions: Extensions::new(),
            extra: None,
        }
    }

    fn mock_settle_failure() -> SettleResponse {
        SettleResponse::Failure {
            reason: r402_core::ErrorReason::UnexpectedSettleError,
            message: Some("boom".into()),
            payer: None,
            transaction: "".into(),
            network: "eip155:1".into(),
            extensions: Extensions::new(),
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
        let req = accepts()
            .into_iter()
            .next()
            .expect("accepts fixture is non-empty");
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

        fn settle_on_cancel<'a>(
            &'a self,
            _: &'a VerifiedPaymentCanceledContext,
        ) -> impl Future<Output = Option<PaymentRequirements>> + Send + 'a {
            std::future::ready(self.cancel.clone())
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
        let w = wrapper(true, true).await;
        let params = CallToolRequestParams::new("demo");
        let result = w
            .invoke(params, |_| async {
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
        let w = wrapper(true, true).await;
        let params =
            attach_payment_to_params(CallToolRequestParams::new("demo"), &sample_payload());
        let result = w
            .invoke(params, |_| async {
                CallToolResult::success(vec![ContentBlock::text("ok")])
            })
            .await;
        assert!(!result.is_error.unwrap_or(false));
        let settle = extract_settle_response(&result).unwrap();
        assert!(settle.is_success());
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
        let params =
            attach_payment_to_params(CallToolRequestParams::new("demo"), &sample_payload());
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
        let w = wrapper(false, true).await;
        let params =
            attach_payment_to_params(CallToolRequestParams::new("demo"), &sample_payload());
        let result = w
            .invoke(params, |_| async {
                CallToolResult::success(vec![ContentBlock::text("nope")])
            })
            .await;
        assert_eq!(result.is_error, Some(true));
        assert!(result.structured_content.is_some());
    }

    #[tokio::test]
    async fn settle_failure_uses_payment_required_format() {
        let w = wrapper(true, false).await;
        let params =
            attach_payment_to_params(CallToolRequestParams::new("demo"), &sample_payload());
        let result = w
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
        let params =
            attach_payment_to_params(CallToolRequestParams::new("demo"), &sample_payload());
        let result = w.invoke(params, never_run_tool).await;
        assert_eq!(settles.settles.load(Ordering::SeqCst), 1);
        assert!(!result.is_error.unwrap_or(false));
        assert!(extract_settle_response(&result).is_some());
        let sc = result
            .structured_content
            .as_ref()
            .expect("skip body as structured");
        assert_eq!(sc.get("skipped"), Some(&json!(true)));
    }

    #[test]
    fn empty_accepts_rejected() {
        let err = PaymentWrapperConfig::try_new(vec![], None).unwrap_err();
        assert!(matches!(err, PaymentWrapperConfigError::EmptyAccepts));
    }

    #[tokio::test]
    async fn invoke_matches_pipeline_accepts_not_raw_config() {
        let fac = MockFacilitator::new(true, true);
        let server = ResourceServer::new(fac);
        let advertised = PaymentRequirements::new(
            "exact".into(),
            "eip155:1".parse().unwrap(),
            "1".into(),
            "0xa".into(),
            "0xb".into(),
            60,
        )
        .with_extra(json!({"feePayer": "payer"}));
        let config = PaymentWrapperConfig::try_new(vec![advertised], None).unwrap();
        let server = server.with_scheme(eip155_wildcard(), FlowScheme::authorization());
        let w = PaymentWrapper::try_new(server, config).await.unwrap();

        let omitted = sample_payload();
        let omitted_params = attach_payment_to_params(CallToolRequestParams::new("demo"), &omitted);
        let omitted_result = w
            .invoke(omitted_params, |_| async {
                CallToolResult::success(vec![ContentBlock::text("should not run")])
            })
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
        let echoed_params = attach_payment_to_params(CallToolRequestParams::new("demo"), &echoed);
        let echoed_result = w
            .invoke(echoed_params, |_| async {
                CallToolResult::success(vec![ContentBlock::text("ok")])
            })
            .await;
        assert!(!echoed_result.is_error.unwrap_or(false));
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
        let settle = extract_settle_response(&result).unwrap();
        assert!(settle.is_success());
    }

    #[tokio::test]
    async fn escrow_success_settles_before_and_after() {
        let fac = MockFacilitator::new(true, true);
        let settles = Arc::clone(&fac);
        let requirements = accepts().into_iter().next().unwrap();
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
        let requirements = accepts().into_iter().next().unwrap();
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
        });
        let requirements = accepts().into_iter().next().unwrap();
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
}
