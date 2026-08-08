//! Client-side MCP auto-pay — Go `X402MCPClient` / `CallPaidTool`.
//!
//! V1 payment paths are intentionally omitted (r402 is V2-only).

use std::future::Future;
use std::sync::Arc;

use r402_core::wire::{PaymentRequired, SettleResponse};
use rmcp::model::{CallToolRequestParams, CallToolResult};
use serde_json::{Map, Value};

use crate::encode::{
    McpPaymentPayload, attach_payment_to_params, extract_payment_required, extract_settle_response,
    is_payment_required_result,
};
use crate::error::{McpClientError, PaymentRequiredError};

/// Minimal tool-call surface (Go `MCPCaller`).
pub trait McpToolCaller: Send + Sync {
    /// Invokes `tools/call`.
    fn call_tool(
        &self,
        params: CallToolRequestParams,
    ) -> impl Future<Output = Result<CallToolResult, String>> + Send;
}

/// Signs a [`PaymentRequired`] into a wire payment payload.
pub trait PaymentSigner: Send + Sync {
    /// Creates a payment for the challenge.
    fn sign_payment(
        &self,
        required: PaymentRequired,
    ) -> impl Future<Output = Result<McpPaymentPayload, String>> + Send;
}

/// Result of a paid tool call (Go `MCPToolCallResult` / `ToolCallResult`).
#[derive(Debug, Clone)]
pub struct PaidToolCallResult {
    /// Underlying MCP tool result.
    pub result: CallToolResult,
    /// Whether a payment was submitted.
    pub payment_made: bool,
    /// Settlement from `_meta`, if present.
    pub payment_response: Option<SettleResponse>,
}

/// Client options (Go `Options`).
#[derive(Debug, Clone, Copy)]
pub struct X402McpClientOptions {
    /// Auto-create payment when challenged (default true).
    pub auto_payment: bool,
}

impl Default for X402McpClientOptions {
    fn default() -> Self {
        Self { auto_payment: true }
    }
}

/// Client hooks (Go hook fields).
#[derive(Clone, Default)]
pub struct ClientHooks {
    /// Can abort or supply a custom payload.
    pub on_payment_required:
        Option<Arc<dyn Fn(PaymentRequiredContext) -> PaymentRequiredHookResult + Send + Sync>>,
    /// Approve/deny auto-pay.
    pub on_payment_requested: Option<Arc<dyn Fn(PaymentRequiredContext) -> bool + Send + Sync>>,
    /// Before signing.
    pub on_before_payment: Option<Arc<dyn Fn(PaymentRequiredContext) + Send + Sync>>,
    /// After paid call returns.
    pub on_after_payment: Option<Arc<dyn Fn(AfterPaymentContext) + Send + Sync>>,
}

impl std::fmt::Debug for ClientHooks {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ClientHooks").finish_non_exhaustive()
    }
}

/// Context for payment-required hooks.
#[derive(Debug, Clone)]
pub struct PaymentRequiredContext {
    /// Tool name.
    pub tool_name: String,
    /// Arguments.
    pub arguments: Map<String, Value>,
    /// Challenge body.
    pub payment_required: PaymentRequired,
}

/// Result of `on_payment_required`.
#[derive(Debug, Clone, Default)]
pub struct PaymentRequiredHookResult {
    /// Abort without paying.
    pub abort: bool,
    /// Optional custom signed payload.
    pub payment: Option<McpPaymentPayload>,
}

/// Context after a paid call.
#[derive(Debug, Clone)]
pub struct AfterPaymentContext {
    /// Tool name.
    pub tool_name: String,
    /// Payload that was submitted.
    pub payment_payload: McpPaymentPayload,
    /// Tool result.
    pub result: CallToolResult,
    /// Settlement if present.
    pub settle_response: Option<SettleResponse>,
}

/// Go `X402MCPClient`.
pub struct X402McpClient<C, S> {
    caller: C,
    signer: S,
    options: X402McpClientOptions,
    hooks: ClientHooks,
}

impl<C, S> std::fmt::Debug for X402McpClient<C, S> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("X402McpClient")
            .field("options", &self.options)
            .finish_non_exhaustive()
    }
}

impl<C, S> X402McpClient<C, S>
where
    C: McpToolCaller,
    S: PaymentSigner,
{
    /// Creates a client with default options.
    #[must_use]
    pub fn new(caller: C, signer: S) -> Self {
        Self {
            caller,
            signer,
            options: X402McpClientOptions::default(),
            hooks: ClientHooks::default(),
        }
    }

    /// Sets options.
    #[must_use]
    pub const fn with_options(mut self, options: X402McpClientOptions) -> Self {
        self.options = options;
        self
    }

    /// Sets hooks.
    #[must_use]
    pub fn with_hooks(mut self, hooks: ClientHooks) -> Self {
        self.hooks = hooks;
        self
    }

    /// Go `CallTool` — automatic payment when required.
    ///
    /// # Errors
    ///
    /// Transport, signing, user deny, or persistent 402.
    pub async fn call_tool(
        &self,
        name: impl Into<String>,
        arguments: Option<Map<String, Value>>,
    ) -> Result<PaidToolCallResult, McpClientError> {
        let name = name.into();
        let args = arguments.unwrap_or_default();
        let mut params = CallToolRequestParams::new(name.clone());
        if !args.is_empty() {
            params = params.with_arguments(args.clone());
        }

        let first = self
            .caller
            .call_tool(params.clone())
            .await
            .map_err(McpClientError::Transport)?;

        if !is_payment_required_result(&first) {
            return Ok(PaidToolCallResult {
                payment_response: extract_settle_response(&first),
                result: first,
                payment_made: false,
            });
        }

        let required = extract_payment_required(&first)
            .ok_or_else(|| McpClientError::Payment("missing PaymentRequired body".into()))?;

        let pr_ctx = PaymentRequiredContext {
            tool_name: name.clone(),
            arguments: args,
            payment_required: required.clone(),
        };

        if let Some(ref hook) = self.hooks.on_payment_required {
            let hr = hook(pr_ctx.clone());
            if hr.abort {
                return Err(PaymentRequiredError::new("Payment required", required).into());
            }
            if let Some(payload) = hr.payment {
                return self
                    .call_tool_with_payment(name, params.arguments.clone(), payload)
                    .await;
            }
        }

        if !self.options.auto_payment {
            return Err(PaymentRequiredError::new("Payment required", required).into());
        }

        if let Some(ref requested) = self.hooks.on_payment_requested
            && !requested(pr_ctx.clone())
        {
            return Err(PaymentRequiredError::new("Payment denied by user", required).into());
        }

        if let Some(ref before) = self.hooks.on_before_payment {
            before(pr_ctx);
        }

        let payload = self
            .signer
            .sign_payment(required)
            .await
            .map_err(McpClientError::Payment)?;

        self.call_tool_with_payment(name, params.arguments, payload)
            .await
    }

    /// Go `CallToolWithPayment`.
    ///
    /// # Errors
    ///
    /// Transport failure or still-required payment.
    pub async fn call_tool_with_payment(
        &self,
        name: impl Into<String>,
        arguments: Option<Map<String, Value>>,
        payload: McpPaymentPayload,
    ) -> Result<PaidToolCallResult, McpClientError> {
        let name = name.into();
        let mut params = CallToolRequestParams::new(name.clone());
        if let Some(args) = arguments {
            params = params.with_arguments(args);
        }
        let params = attach_payment_to_params(params, &payload);

        let result = self
            .caller
            .call_tool(params)
            .await
            .map_err(McpClientError::Transport)?;

        if is_payment_required_result(&result) {
            return Err(McpClientError::StillRequired);
        }

        let settle = extract_settle_response(&result);
        if let Some(ref after) = self.hooks.on_after_payment {
            after(AfterPaymentContext {
                tool_name: name,
                payment_payload: payload,
                result: result.clone(),
                settle_response: settle.clone(),
            });
        }

        Ok(PaidToolCallResult {
            payment_response: settle,
            result,
            payment_made: true,
        })
    }

    /// Go `GetToolPaymentRequirements`.
    ///
    /// # Errors
    ///
    /// Transport failures.
    pub async fn get_tool_payment_requirements(
        &self,
        name: impl Into<String>,
        arguments: Option<Map<String, Value>>,
    ) -> Result<Option<PaymentRequired>, McpClientError> {
        let mut params = CallToolRequestParams::new(name.into());
        if let Some(args) = arguments {
            params = params.with_arguments(args);
        }
        let result = self
            .caller
            .call_tool(params)
            .await
            .map_err(McpClientError::Transport)?;
        Ok(extract_payment_required(&result))
    }
}

/// Go `CallPaidTool` free function.
///
/// # Errors
///
/// Same as [`X402McpClient::call_tool`].
pub async fn call_paid_tool<C, S>(
    caller: C,
    signer: S,
    name: impl Into<String>,
    arguments: Option<Map<String, Value>>,
) -> Result<PaidToolCallResult, McpClientError>
where
    C: McpToolCaller,
    S: PaymentSigner,
{
    X402McpClient::new(caller, signer)
        .call_tool(name, arguments)
        .await
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use r402_core::wire::{PaymentRequirements, ResourceInfo};
    use serde_json::json;

    use super::*;
    use crate::encode::payment_required_tool_result;

    struct MockCaller {
        calls: Mutex<u8>,
    }

    impl McpToolCaller for MockCaller {
        fn call_tool(
            &self,
            params: CallToolRequestParams,
        ) -> impl Future<Output = Result<CallToolResult, String>> + Send {
            std::future::ready(self.handle_call(&params))
        }
    }

    impl MockCaller {
        fn handle_call(&self, params: &CallToolRequestParams) -> Result<CallToolResult, String> {
            let unpaid = params.meta.is_none();
            let mut n = self.calls.lock().map_err(|e| e.to_string())?;
            *n = n.saturating_add(1);
            drop(n);
            if !unpaid {
                return Ok(CallToolResult::success(vec![
                    rmcp::model::ContentBlock::text("ok"),
                ]));
            }
            let network = "eip155:1"
                .parse()
                .map_err(|e| format!("fixture network: {e}"))?;
            let resource = ResourceInfo::new("mcp://tool/demo");
            let req = PaymentRequirements::new(
                "exact".into(),
                network,
                "1".into(),
                "0xa".into(),
                "0xb".into(),
                60,
            );
            let pr = PaymentRequired::new(resource).with_accepts(vec![req]);
            Ok(payment_required_tool_result(&pr))
        }
    }

    struct MockSigner;

    impl PaymentSigner for MockSigner {
        fn sign_payment(
            &self,
            required: PaymentRequired,
        ) -> impl Future<Output = Result<McpPaymentPayload, String>> + Send {
            let result = required
                .accepts
                .into_iter()
                .next()
                .ok_or_else(|| "no accepts".to_owned())
                .map(|accepted| McpPaymentPayload::new(accepted, json!({"s": 1})));
            std::future::ready(result)
        }
    }

    #[tokio::test]
    async fn auto_pays_on_second_call() {
        let client = X402McpClient::new(
            MockCaller {
                calls: Mutex::new(0),
            },
            MockSigner,
        );
        let out = client.call_tool("demo", None).await.unwrap();
        assert!(out.payment_made);
        assert!(!out.result.is_error.unwrap_or(false));
    }

    #[tokio::test]
    async fn auto_payment_disabled_returns_402() {
        let client = X402McpClient::new(
            MockCaller {
                calls: Mutex::new(0),
            },
            MockSigner,
        )
        .with_options(X402McpClientOptions {
            auto_payment: false,
        });
        let err = client.call_tool("demo", None).await.unwrap_err();
        match err {
            McpClientError::PaymentRequired(e) => assert_eq!(e.code, 402),
            other => panic!("expected PaymentRequired, got {other:?}"),
        }
    }
}
