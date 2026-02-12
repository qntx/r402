//! Server-side MCP x402 payment wrapper.
//!
//! This module provides [`PaymentWrapper`] which wraps MCP tool handlers
//! with automatic x402 payment verification and settlement.
//!
//! # Flow
//!
//! 1. Extract `x402/payment` from request `_meta`
//! 2. If no payment, return 402 payment required error
//! 3. Verify payment via facilitator
//! 4. `on_before_execution` hook (can abort)
//! 5. Execute the original handler
//! 6. `on_after_execution` hook
//! 7. Settle payment via facilitator
//! 8. `on_after_settlement` hook
//! 9. Return result with settlement info in `_meta`

use std::future::Future;
use std::sync::Arc;

use r402::facilitator::Facilitator;
use r402::proto;
use r402::proto::v2;
use serde_json::Value;

use crate::PAYMENT_RESPONSE_META_KEY;
use crate::error::McpPaymentError;
use crate::extract::{self, wrap_x402_error_envelope};
use crate::types::{
    AfterExecutionContext, CallToolParams, CallToolResult, ContentItem, NoServerHooks,
    PaymentWrapperConfig, ServerHookContext, ServerHooks, SettlementContext,
};

/// Wraps MCP tool handlers with x402 payment verification and settlement.
///
/// The wrapper intercepts tool call requests, enforces payment, and
/// manages the full verify → execute → settle lifecycle.
///
/// # Examples
///
/// ```rust,ignore
/// let wrapper = PaymentWrapper::new(facilitator, PaymentWrapperConfig {
///     accepts: vec![payment_requirements],
///     resource: Some(resource_info),
///     ..Default::default()
/// });
///
/// let result = wrapper.process(request, |req| async {
///     Ok(CallToolResult { content: vec![ContentItem::text("ok")], ..Default::default() })
/// }).await;
/// ```
pub struct PaymentWrapper {
    facilitator: Arc<dyn Facilitator>,
    config: PaymentWrapperConfig,
}

impl std::fmt::Debug for PaymentWrapper {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PaymentWrapper")
            .field("config", &self.config)
            .finish_non_exhaustive()
    }
}

impl PaymentWrapper {
    /// Creates a new payment wrapper.
    ///
    /// # Panics
    ///
    /// Panics if `config.accepts` is empty.
    pub fn new(facilitator: Arc<dyn Facilitator>, config: PaymentWrapperConfig) -> Self {
        assert!(
            !config.accepts.is_empty(),
            "PaymentWrapperConfig.accepts must have at least one payment requirement"
        );
        Self {
            facilitator,
            config,
        }
    }

    /// Processes a tool call request with payment enforcement.
    ///
    /// The `handler` closure is called only after payment verification succeeds.
    /// Settlement occurs after the handler returns a successful result.
    ///
    /// # Errors
    ///
    /// Returns a [`CallToolResult`] with `is_error: true` for payment failures
    /// (following MCP convention of returning tool errors, not transport errors).
    pub async fn process<H, Fut>(&self, request: CallToolParams, handler: H) -> CallToolResult
    where
        H: FnOnce(CallToolParams) -> Fut,
        Fut: Future<Output = Result<CallToolResult, McpPaymentError>>,
    {
        // Extract payment from _meta
        let payment_data = request
            .meta
            .as_ref()
            .and_then(extract::extract_payment_from_meta);

        let Some(payment_value) = payment_data else {
            return self.payment_required_result("Payment Required");
        };

        // Deserialize to create verify request
        let requirements = &self.config.accepts[0];
        let verify_request = match build_verify_request(&payment_value, requirements) {
            Ok(req) => req,
            Err(msg) => return self.payment_required_result(&msg),
        };

        // Verify payment
        let verify_result = self.facilitator.verify(verify_request.clone()).await;
        let verify_response = match verify_result {
            Ok(resp) => resp,
            Err(e) => {
                return self.payment_required_result(&format!("Payment verification error: {e}"));
            }
        };

        if !verify_response.is_valid() {
            let reason = match &verify_response {
                proto::VerifyResponse::Invalid { reason, .. } => reason.as_str(),
                _ => "unknown",
            };
            return self.payment_required_result(&format!("Payment verification failed: {reason}"));
        }

        // Parse arguments for hooks
        let arguments = request.arguments.clone();
        let hooks = self.hooks();

        // on_before_execution hook
        let hook_ctx = ServerHookContext {
            tool_name: request.name.clone(),
            arguments: arguments.clone(),
            payment_requirements: requirements.clone(),
            payment_payload: payment_value.clone(),
        };

        match hooks.on_before_execution(&hook_ctx).await {
            Ok(true) => {}
            Ok(false) => {
                return self
                    .payment_required_result("Execution aborted by on_before_execution hook");
            }
            Err(e) => {
                return self.payment_required_result(&format!("Before execution hook error: {e}"));
            }
        }

        // Execute the original handler
        let result = match handler(request).await {
            Ok(r) => r,
            Err(e) => {
                return CallToolResult {
                    content: vec![ContentItem::text(e.to_string())],
                    is_error: true,
                    ..Default::default()
                };
            }
        };

        // If handler returned an error, don't settle
        if result.is_error {
            return result;
        }

        // on_after_execution hook (non-fatal)
        let after_exec_ctx = AfterExecutionContext {
            server_ctx: hook_ctx.clone(),
            result: result.clone(),
        };
        let _ = hooks.on_after_execution(&after_exec_ctx).await;

        // Settle payment
        let settle_request = proto::SettleRequest::from(verify_request);
        let settle_result = self.facilitator.settle(settle_request).await;
        let settle_response = match settle_result {
            Ok(resp) => resp,
            Err(e) => {
                return self.payment_required_result(&format!("Settlement error: {e}"));
            }
        };

        if !settle_response.is_success() {
            let reason = match &settle_response {
                proto::SettleResponse::Error { reason, .. } => reason.as_str(),
                _ => "unknown",
            };
            return self.payment_required_result(&format!("Settlement failed: {reason}"));
        }

        // on_after_settlement hook (non-fatal)
        let settle_ctx = SettlementContext {
            server_ctx: hook_ctx,
            settlement: settle_response.clone(),
        };
        let _ = hooks.on_after_settlement(&settle_ctx).await;

        // Attach settlement response to result _meta
        let mut result_meta = result.meta.unwrap_or_default();
        if let Ok(settle_value) = serde_json::to_value(&settle_response) {
            result_meta.insert(PAYMENT_RESPONSE_META_KEY.to_owned(), settle_value);
        }

        CallToolResult {
            content: result.content,
            is_error: result.is_error,
            meta: Some(result_meta),
            structured_content: result.structured_content,
        }
    }

    /// Creates a 402 payment required error result.
    ///
    /// Uses the TS-compatible `x402/error` envelope format for cross-language
    /// interoperability. The envelope is placed in both `content[0].text` and
    /// `structuredContent`, with `isError: true`.
    fn payment_required_result(&self, error_msg: &str) -> CallToolResult {
        let resource = self
            .config
            .resource
            .clone()
            .unwrap_or_else(|| v2::ResourceInfo {
                url: "mcp://tool/unknown".to_owned(),
                description: "Unknown tool".to_owned(),
                mime_type: "application/json".to_owned(),
            });

        let pr = v2::PaymentRequired {
            x402_version: v2::V2,
            error: Some(error_msg.to_owned()),
            resource,
            accepts: self.config.accepts.clone(),
            extensions: self
                .config
                .extensions
                .as_ref()
                .map(|ext| ext.iter().map(|(k, v)| (k.clone(), v.clone())).collect()),
        };

        let envelope = wrap_x402_error_envelope(&pr).unwrap_or_default();
        let text = serde_json::to_string(&envelope).unwrap_or_default();

        CallToolResult {
            content: vec![ContentItem::text(text)],
            is_error: true,
            meta: None,
            structured_content: Some(envelope),
        }
    }

    fn hooks(&self) -> &dyn ServerHooks {
        self.config.hooks.as_deref().unwrap_or(&NoServerHooks)
    }
}

/// Builds a [`proto::VerifyRequest`] from a payment payload and requirements.
fn build_verify_request(
    payment_value: &Value,
    requirements: &v2::PaymentRequirements,
) -> Result<proto::VerifyRequest, String> {
    let requirements_value =
        serde_json::to_value(requirements).map_err(|e| format!("Invalid requirements: {e}"))?;

    let verify_json = serde_json::json!({
        "x402Version": 2,
        "paymentPayload": payment_value,
        "paymentRequirements": requirements_value,
    });

    Ok(proto::VerifyRequest::from(verify_json))
}
