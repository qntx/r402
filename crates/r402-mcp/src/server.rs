//! Server-side MCP payment wrapper.
//!
//! Control flow ports Go `go/mcp/server.go` `PaymentWrapper.Wrap`:
//! extract meta payment → match accepts → verify → handler → settle → attach meta.
//!
//! Verify/settle failures return **tool-level** error results (not transport errors),
//! matching the official Go SDK.

use std::future::Future;
use std::sync::Arc;

use r402_core::facilitator::Facilitator;
use r402_core::wire::{
    PaymentRequired, PaymentRequirements, ResourceInfo, SettleRequest, TypedVerifyRequest,
    VerifyRequest, V2, find_matching_requirements,
};
use rmcp::model::{CallToolRequestParams, CallToolResult};

use crate::encode::{
    McpPaymentPayload, attach_settle_response, extract_payment_from_params,
    payment_required_tool_result, settlement_failed_tool_result,
};

/// Server-side lifecycle hooks (optional).
#[derive(Clone, Default)]
pub struct PaymentWrapperHooks {
    /// Called after successful verify, before the tool runs.
    /// Return `false` to abort (tool-level payment error).
    pub on_before_execution: Option<Arc<dyn Fn(ServerHookContext) -> bool + Send + Sync>>,
    /// Called after the tool succeeds, before settle.
    pub on_after_execution: Option<Arc<dyn Fn(ServerHookContext) + Send + Sync>>,
    /// Called after successful settlement.
    pub on_after_settlement: Option<Arc<dyn Fn(ServerHookContext) + Send + Sync>>,
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

/// Context passed to server hooks.
#[derive(Debug, Clone)]
pub struct ServerHookContext {
    /// Tool name from the MCP call.
    pub tool_name: String,
    /// Matched payment requirements.
    pub requirements: PaymentRequirements,
}

/// Configuration for [`PaymentWrapper`].
#[derive(Debug, Clone)]
pub struct PaymentWrapperConfig {
    /// Advertised payment options (Go `Accepts`).
    pub accepts: Vec<PaymentRequirements>,
    /// Resource metadata for payment-required responses.
    pub resource: ResourceInfo,
    /// Optional lifecycle hooks.
    pub hooks: PaymentWrapperHooks,
}

impl PaymentWrapperConfig {
    /// Builds config with accepts and resource.
    #[must_use]
    pub fn new(accepts: Vec<PaymentRequirements>, resource: ResourceInfo) -> Self {
        Self {
            accepts,
            resource,
            hooks: PaymentWrapperHooks::default(),
        }
    }

    /// Attaches hooks.
    #[must_use]
    pub fn with_hooks(mut self, hooks: PaymentWrapperHooks) -> Self {
        self.hooks = hooks;
        self
    }
}

/// Wraps MCP tool handlers with x402 verify / settle.
pub struct PaymentWrapper<F> {
    facilitator: Arc<F>,
    config: PaymentWrapperConfig,
}

impl<F> std::fmt::Debug for PaymentWrapper<F> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PaymentWrapper")
            .field("config", &self.config)
            .finish_non_exhaustive()
    }
}

impl<F> PaymentWrapper<F>
where
    F: Facilitator + 'static,
{
    /// Creates a wrapper around any [`Facilitator`].
    #[must_use]
    pub const fn new(facilitator: Arc<F>, config: PaymentWrapperConfig) -> Self {
        Self {
            facilitator,
            config,
        }
    }

    /// Builds a payment-required tool result for `error_msg`.
    #[must_use]
    pub fn payment_required_result(&self, error_msg: impl Into<String>) -> CallToolResult {
        let required = PaymentRequired::new(self.config.resource.clone())
            .with_error(error_msg.into())
            .with_accepts(self.config.accepts.clone());
        payment_required_tool_result(&required)
    }

    /// Runs the full verify → handler → settle pipeline.
    ///
    /// `handler` receives the original params (after payment extraction).
    pub async fn invoke<H, Fut>(&self, params: CallToolRequestParams, handler: H) -> CallToolResult
    where
        H: FnOnce(CallToolRequestParams) -> Fut,
        Fut: Future<Output = CallToolResult>,
    {
        let Some(payload) = extract_payment_from_params(&params) else {
            return self.payment_required_result("Payment Required");
        };

        let Some(requirements) =
            find_matching_requirements(&self.config.accepts, &payload.accepted).cloned()
        else {
            return self.payment_required_result("No matching payment requirements found");
        };

        let verify_req = match build_verify_request(&payload, &requirements) {
            Ok(r) => r,
            Err(msg) => return self.payment_required_result(msg),
        };

        match self.facilitator.verify(verify_req.clone()).await {
            Ok(resp) if resp.is_valid() => {}
            Ok(_) => {
                return self.payment_required_result("Payment verification failed");
            }
            Err(err) => {
                return self.payment_required_result(format!("Payment verification error: {err}"));
            }
        }

        let tool_name = params.name.to_string();
        let hook_ctx = ServerHookContext {
            tool_name,
            requirements: requirements.clone(),
        };

        if let Some(ref before) = self.config.hooks.on_before_execution
            && !before(hook_ctx.clone())
        {
            return self.payment_required_result("Execution aborted by OnBeforeExecution hook");
        }

        let result = handler(params).await;
        if result.is_error.unwrap_or(false) {
            return result;
        }

        if let Some(ref after) = self.config.hooks.on_after_execution {
            after(hook_ctx.clone());
        }

        let settle_req = SettleRequest::from(verify_req.into_json());
        let settle = match self.facilitator.settle(settle_req).await {
            Ok(s) if s.is_success() => s,
            Ok(_) => return settlement_failed_tool_result("Settlement failed"),
            Err(err) => {
                return settlement_failed_tool_result(format!("Settlement error: {err}"));
            }
        };

        if let Some(ref after_settle) = self.config.hooks.on_after_settlement {
            after_settle(hook_ctx);
        }

        attach_settle_response(result, &settle)
    }
}

fn build_verify_request(
    payload: &McpPaymentPayload,
    requirements: &PaymentRequirements,
) -> Result<VerifyRequest, String> {
    let typed: TypedVerifyRequest<2, McpPaymentPayload, PaymentRequirements> =
        TypedVerifyRequest {
            x402_version: V2,
            payment_payload: payload.clone(),
            payment_requirements: requirements.clone(),
        };
    let json = serde_json::to_value(&typed).map_err(|e| e.to_string())?;
    Ok(VerifyRequest::from(json))
}
