//! Server-side MCP payment wrapper.
//!
//! Ports Go `go/mcp/server.go` `PaymentWrapper.Wrap` control flow:
//! extract meta payment → match accepts → verify → hooks → handler → settle → meta.
//!
//! Verify/settle failures return **tool-level** dual-format payment-required
//! results (not transport errors), matching the official Go SDK.

use std::future::Future;
use std::sync::Arc;

use r402_core::resource_server::{ResourceServer, SkipHandlerDirective};
use r402_core::wire::{Extensions, PaymentRequired, PaymentRequirements, ResourceInfo};
use rmcp::model::{CallToolRequestParams, CallToolResult, ContentBlock};
use serde_json::Value;

use crate::encode::{
    McpPaymentPayload, attach_settle_response, extract_payment_from_params,
    payment_required_tool_result, settlement_failed_tool_result,
};

/// Error constructing a [`PaymentWrapper`].
#[derive(Debug, Clone, Copy, thiserror::Error)]
pub enum PaymentWrapperConfigError {
    /// `accepts` must be non-empty (Go panics; we return an error).
    #[error("PaymentWrapperConfig.accepts must have at least one payment requirement")]
    EmptyAccepts,
}

/// Builds a successful tool result from a [`SkipHandlerDirective`].
///
/// JSON bodies become `structuredContent`; string bodies become text content;
/// empty directives yield a `null` structured success (mirrors HTTP JSON body).
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
    pub settlement: r402_core::wire::SettleResponse,
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
    /// # Errors
    ///
    /// Empty accepts.
    pub fn try_new(
        server: ResourceServer,
        config: PaymentWrapperConfig,
    ) -> Result<Self, PaymentWrapperConfigError> {
        if config.accepts.is_empty() {
            return Err(PaymentWrapperConfigError::EmptyAccepts);
        }
        Ok(Self { server, config })
    }

    /// Default resource when config omits one (Go fallback).
    fn resource_or_default(&self) -> ResourceInfo {
        self.config.resource.clone().unwrap_or_else(|| {
            ResourceInfo::new("mcp://tool/unknown")
                .with_description("Unknown tool")
                .with_mime_type("application/json")
        })
    }

    /// Go `paymentRequiredResult`.
    #[must_use]
    pub fn payment_required_result(&self, error_msg: impl Into<String>) -> CallToolResult {
        let mut required = PaymentRequired::new(self.resource_or_default())
            .with_error(error_msg.into())
            .with_accepts(self.config.accepts.clone());
        if !self.config.extensions.is_empty() {
            required = required.with_extensions(self.config.extensions.clone());
        }
        payment_required_tool_result(&required)
    }

    /// Go `settlementFailedResult` (R5: same dual format as payment-required).
    #[must_use]
    pub fn settlement_failed_result(&self, error_msg: impl Into<String>) -> CallToolResult {
        settlement_failed_tool_result(
            &self.config.accepts,
            &self.resource_or_default(),
            &self.config.extensions,
            error_msg,
        )
    }

    /// Full pipeline (Go `Wrap` body). Prefer [`Self::wrap`] for rmcp registration.
    pub async fn invoke<H, Fut>(&self, params: CallToolRequestParams, handler: H) -> CallToolResult
    where
        H: FnOnce(CallToolRequestParams) -> Fut,
        Fut: Future<Output = CallToolResult>,
    {
        let Some(payload) = extract_payment_from_params(&params) else {
            return self.payment_required_result("Payment Required");
        };

        let Some(requirements) = self
            .server
            .find_matching_requirements(&self.config.accepts, &payload)
            .cloned()
        else {
            return self.payment_required_result("No matching payment requirements found");
        };

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
                    .payment_required_result(format!("Payment verification failed: {reason}"));
            }
            Err(err) => {
                return self.payment_required_result(format!("Payment verification error: {err}"));
            }
        };

        let cancel = self
            .server
            .cancellation_guard(payload.clone(), requirements.clone());

        let arguments = params.arguments.clone().unwrap_or_default();
        let tool_name = params.name.to_string();
        let hook_ctx = ServerHookContext {
            tool_name,
            arguments,
            payment_requirements: requirements.clone(),
            payment_payload: payload.clone(),
        };

        // SkipHandler: settle without running the tool (official after-verify path).
        let result = if let Some(ref directive) = verify_out.skip_handler {
            skip_handler_tool_result(directive)
        } else {
            if let Some(ref before) = self.config.hooks.on_before_execution
                && !before(hook_ctx.clone())
            {
                cancel
                    .cancel(
                        r402_core::CancelReason::AfterVerifyAborted,
                        Some("Execution aborted by OnBeforeExecution hook"),
                        None,
                    )
                    .await;
                return self.payment_required_result("Execution aborted by OnBeforeExecution hook");
            }

            let result = handler(params).await;
            if result.is_error.unwrap_or(false) {
                cancel
                    .cancel(
                        r402_core::CancelReason::HandlerFailed,
                        Some("tool returned isError"),
                        None,
                    )
                    .await;
                return result;
            }
            result
        };

        if let Some(ref after) = self.config.hooks.on_after_execution {
            after(AfterExecutionContext {
                server: hook_ctx.clone(),
                result: result.clone(),
            });
        }

        let settle = match self
            .server
            .settle_payment(&payload, &requirements, None)
            .await
        {
            Ok(s) if s.is_success() => s,
            Ok(s) => {
                let reason = match s {
                    r402_core::wire::SettleResponse::Failure {
                        reason, message, ..
                    } => message.map_or_else(|| reason.to_string(), |m| m.to_string()),
                    // Success handled above; `_` for non_exhaustive future variants.
                    _ => "Settlement failed".into(),
                };
                return self.settlement_failed_result(format!("Settlement failed: {reason}"));
            }
            Err(err) => {
                return self.settlement_failed_result(format!("Settlement error: {err}"));
            }
        };

        if let Some(ref after_settle) = self.config.hooks.on_after_settlement {
            after_settle(SettlementContext {
                server: hook_ctx,
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
    use std::future::Future;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use r402_core::FacilitatorError;
    use r402_core::facilitator::Facilitator;
    use r402_core::resource_server::ResourceServerHooks;
    use r402_core::wire::{
        PaymentRequirements, ResourceInfo, SettleRequest, SettleResponse, SupportedResponse,
        VerifyRequest, VerifyResponse,
    };
    use rmcp::model::ContentBlock;
    use serde_json::json;

    use super::*;
    use crate::encode::{McpPaymentPayload, attach_payment_to_params, extract_settle_response};

    struct MockFacilitator {
        verifies: AtomicUsize,
        settles: AtomicUsize,
        verify_ok: bool,
        settle_ok: bool,
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
            self.settles.fetch_add(1, Ordering::SeqCst);
            std::future::ready(if self.settle_ok {
                Ok(SettleResponse::Success {
                    payer: "0xpayer".into(),
                    transaction: "0xtx".into(),
                    network: "eip155:1".into(),
                    amount: Some("1".into()),
                    extensions: Extensions::new(),
                    extra: None,
                })
            } else {
                Ok(SettleResponse::Failure {
                    reason: r402_core::ErrorReason::UnexpectedSettleError,
                    message: Some("boom".into()),
                    payer: None,
                    transaction: Default::default(),
                    network: "eip155:1".into(),
                    extensions: Extensions::new(),
                    extra: None,
                })
            })
        }

        fn supported(
            &self,
        ) -> impl Future<Output = Result<SupportedResponse, FacilitatorError>> + Send {
            std::future::ready(Ok(SupportedResponse::default()))
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

    fn wrapper(verify_ok: bool, settle_ok: bool) -> PaymentWrapper {
        let fac = Arc::new(MockFacilitator {
            verifies: AtomicUsize::new(0),
            settles: AtomicUsize::new(0),
            verify_ok,
            settle_ok,
        });
        let server = ResourceServer::new(fac);
        let config = PaymentWrapperConfig::try_new(
            accepts(),
            Some(ResourceInfo::new("mcp://tool/demo").with_description("Demo")),
        )
        .unwrap();
        PaymentWrapper::try_new(server, config).unwrap()
    }

    #[tokio::test]
    async fn missing_payment_returns_dual_format_402() {
        let w = wrapper(true, true);
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
        let w = wrapper(true, true);
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
        let fac = Arc::new(MockFacilitator {
            verifies: AtomicUsize::new(0),
            settles: AtomicUsize::new(0),
            verify_ok: true,
            settle_ok: true,
        });
        let settles = Arc::clone(&fac);
        let server = ResourceServer::new(fac);
        let config = PaymentWrapperConfig::try_new(accepts(), None).unwrap();
        let w = PaymentWrapper::try_new(server, config).unwrap();
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
        let w = wrapper(false, true);
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
        let w = wrapper(true, false);
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
            _ctx: &'a r402_core::VerifyResultContext,
        ) -> impl Future<Output = r402_core::AfterVerifyDecision> + Send + 'a {
            std::future::ready(r402_core::AfterVerifyDecision::SkipHandler {
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
        let fac = Arc::new(MockFacilitator {
            verifies: AtomicUsize::new(0),
            settles: AtomicUsize::new(0),
            verify_ok: true,
            settle_ok: true,
        });
        let settles = Arc::clone(&fac);
        let server = ResourceServer::new(fac).with_hook(SkipHandlerHook);
        let config = PaymentWrapperConfig::try_new(accepts(), None).unwrap();
        let w = PaymentWrapper::try_new(server, config).unwrap();
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
}
