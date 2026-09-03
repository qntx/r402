//! Client-side MCP auto-pay — Go `X402MCPClient` / `CallPaidTool`.

use std::future::Future;
use std::sync::Arc;

use r402_client::{
    ClientHooks, FirstMatch, PaymentClient, PaymentPolicy, PaymentResponseContext, PaymentSelector,
    SchemeClient, SpendControls,
};
use r402_protocol::payment::{Base64Bytes, PaymentRequired, ResourceInfo, SettleResponse};
use rmcp::model::{CallToolRequestParams, CallToolResult};
use serde_json::{Map, Value};

use crate::error::{McpCallError, McpClientError, PaymentRequiredError};
use crate::tool::{
    McpPaymentPayload, attach_payment_to_params, extract_payment_required,
    extract_payment_required_from_rpc, extract_settle_response, payment_payload_from_signed,
};

/// Minimal tool-call surface (Go `MCPCaller`).
pub trait McpToolCaller: Send + Sync {
    /// Invokes `tools/call`.
    fn call_tool(
        &self,
        params: CallToolRequestParams,
    ) -> impl Future<Output = Result<CallToolResult, McpCallError>> + Send;
}

/// Result of a paid tool call (Go `MCPToolCallResult`).
#[derive(Debug, Clone)]
pub struct PaidToolCallResult {
    /// Underlying MCP tool result.
    pub result: CallToolResult,
    /// Whether a payment was submitted.
    pub payment_made: bool,
    /// Settlement from `_meta`, if present.
    pub payment_response: Option<SettleResponse>,
    /// `true` when `on_payment_response` asked for a corrective retry.
    pub recovery_requested: bool,
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

/// MCP client hooks (Go hook fields + core `onPaymentResponse`).
#[derive(Clone, Default)]
pub struct McpClientHooks {
    /// Can abort or supply a custom payload.
    pub on_payment_required:
        Option<Arc<dyn Fn(PaymentRequiredContext) -> PaymentRequiredHookResult + Send + Sync>>,
    /// Approve/deny auto-pay.
    pub on_payment_requested: Option<Arc<dyn Fn(PaymentRequiredContext) -> bool + Send + Sync>>,
    /// Before signing.
    pub on_before_payment: Option<Arc<dyn Fn(PaymentRequiredContext) + Send + Sync>>,
    /// After paid call returns (MCP-specific).
    pub on_after_payment: Option<Arc<dyn Fn(AfterPaymentContext) + Send + Sync>>,
}

impl std::fmt::Debug for McpClientHooks {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("McpClientHooks").finish_non_exhaustive()
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
pub struct X402McpClient<C, S = FirstMatch> {
    caller: C,
    payments: PaymentClient<S>,
    options: X402McpClientOptions,
    hooks: McpClientHooks,
}

impl<C, S> std::fmt::Debug for X402McpClient<C, S> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("X402McpClient")
            .field("options", &self.options)
            .field("payments", &self.payments)
            .finish_non_exhaustive()
    }
}

impl<C> X402McpClient<C, FirstMatch> {
    /// Creates a client with default [`PaymentClient`] (`$1` spend controls).
    #[must_use]
    pub fn new(caller: C) -> Self {
        Self {
            caller,
            payments: PaymentClient::new(),
            options: X402McpClientOptions::default(),
            hooks: McpClientHooks::default(),
        }
    }
}

impl<C, S> X402McpClient<C, S> {
    /// Wraps an existing payment client.
    #[must_use]
    pub fn from_payment_client(caller: C, payments: PaymentClient<S>) -> Self {
        Self {
            caller,
            payments,
            options: X402McpClientOptions::default(),
            hooks: McpClientHooks::default(),
        }
    }

    /// Shared reference to the core payment client.
    #[must_use]
    pub const fn payment_client(&self) -> &PaymentClient<S> {
        &self.payments
    }

    /// Sets options.
    #[must_use]
    pub const fn with_options(mut self, options: X402McpClientOptions) -> Self {
        self.options = options;
        self
    }

    /// Sets MCP-specific hooks.
    #[must_use]
    pub fn with_mcp_hooks(mut self, hooks: McpClientHooks) -> Self {
        self.hooks = hooks;
        self
    }

    /// Registers a scheme client.
    #[must_use]
    pub fn register(mut self, scheme: impl SchemeClient + 'static) -> Self {
        self.payments = self.payments.register(scheme);
        self
    }

    /// Sets a custom payment selector.
    #[must_use]
    pub fn with_selector<P: PaymentSelector>(self, selector: P) -> X402McpClient<C, P> {
        X402McpClient {
            caller: self.caller,
            payments: self.payments.with_selector(selector),
            options: self.options,
            hooks: self.hooks,
        }
    }

    /// Adds a payment policy.
    #[must_use]
    pub fn with_policy(mut self, policy: impl PaymentPolicy + 'static) -> Self {
        self.payments = self.payments.with_policy(policy);
        self
    }

    /// Adds a client lifecycle hook (including `on_payment_response`).
    #[must_use]
    pub fn with_hook(mut self, hook: impl ClientHooks + 'static) -> Self {
        self.payments = self.payments.with_hook(hook);
        self
    }

    /// Enables spend controls with the given configuration.
    #[must_use]
    pub fn with_spend_controls(mut self, controls: SpendControls) -> Self {
        self.payments = self.payments.with_spend_controls(controls);
        self
    }

    /// Disables all spend controls (any asset, no caps).
    #[must_use]
    pub fn disable_spend_controls(mut self) -> Self {
        self.payments = self.payments.disable_spend_controls();
        self
    }
}

impl<C, S> X402McpClient<C, S>
where
    C: McpToolCaller,
    S: PaymentSelector,
{
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

        let (first, required) = interpret_call(self.caller.call_tool(params.clone()).await)?;

        let Some(required) = required else {
            let result = first.ok_or_else(missing_tool_result)?;
            return Ok(PaidToolCallResult {
                payment_response: extract_settle_response(&result),
                result,
                payment_made: false,
                recovery_requested: false,
            });
        };

        let pr_ctx = PaymentRequiredContext {
            tool_name: name.clone(),
            arguments: args,
            payment_required: required.clone(),
        };

        if let Some(ref hook) = self.hooks.on_payment_required {
            let hook_result = hook(pr_ctx.clone());
            if hook_result.abort {
                return Err(PaymentRequiredError::new("Payment required", required).into());
            }
            if let Some(payload) = hook_result.payment {
                return self
                    .call_tool_with_payment_and_challenge(
                        name,
                        params.arguments.clone(),
                        payload,
                        Some(required),
                    )
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

        let created = self.payments.create_payment(&required).await?;
        let payload = payment_payload_from_signed(&created.signed_payload)?;

        self.call_tool_with_payment_and_challenge(name, params.arguments, payload, Some(required))
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
        self.call_tool_with_payment_and_challenge(name, arguments, payload, None)
            .await
    }

    async fn call_tool_with_payment_and_challenge(
        &self,
        name: impl Into<String>,
        arguments: Option<Map<String, Value>>,
        payload: McpPaymentPayload,
        original_challenge: Option<PaymentRequired>,
    ) -> Result<PaidToolCallResult, McpClientError> {
        let name = name.into();
        let mut params = CallToolRequestParams::new(name.clone());
        if let Some(args) = arguments {
            params = params.with_arguments(args);
        }
        let params = attach_payment_to_params(params, &payload);

        let (result, corrective) = interpret_call(self.caller.call_tool(params).await)?;

        let signed_payload = encode_mcp_payload_b64(&payload);
        let settle = result.as_ref().and_then(extract_settle_response);
        let still_required = corrective.is_some();

        let challenge = original_challenge.clone().or_else(|| corrective.clone());
        let recovery_requested = if settle.is_some() || still_required {
            let mut ctx = PaymentResponseContext::new(
                challenge
                    .unwrap_or_else(|| PaymentRequired::new(ResourceInfo::new("mcp://unknown"))),
                signed_payload,
            );
            if let Some(ref settle) = settle {
                ctx = ctx.with_settle_response(settle.clone());
            }
            if let Some(ref corrective) = corrective {
                ctx = ctx.with_corrective_payment_required(corrective.clone());
            }
            self.payments.handle_payment_response(&ctx).await.recovered
        } else {
            false
        };

        if still_required {
            return Err(McpClientError::StillRequired {
                payment_required: corrective.map(Box::new),
                recovery_requested,
            });
        }

        let result = result.ok_or_else(missing_tool_result)?;

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
            recovery_requested,
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
        let (_, required) = interpret_call(self.caller.call_tool(params).await)?;
        Ok(required)
    }
}

fn interpret_call(
    result: Result<CallToolResult, McpCallError>,
) -> Result<(Option<CallToolResult>, Option<PaymentRequired>), McpClientError> {
    match result {
        Ok(result) => {
            let required = extract_payment_required(&result);
            Ok((Some(result), required))
        }
        Err(err) => extract_payment_required_from_rpc(&err)
            .map_or_else(|| Err(err.into()), |required| Ok((None, Some(required)))),
    }
}

fn missing_tool_result() -> McpClientError {
    McpClientError::Transport(
        "mcp tool call returned neither a result nor a payment challenge".into(),
    )
}

fn encode_mcp_payload_b64(payload: &McpPaymentPayload) -> String {
    serde_json::to_vec(payload).map_or_else(
        |_| String::new(),
        |bytes| Base64Bytes::encode(&bytes).to_string(),
    )
}

/// Go `CallPaidTool` free function.
///
/// # Errors
///
/// Same as [`X402McpClient::call_tool`].
pub async fn call_paid_tool<C, S>(
    caller: C,
    payments: PaymentClient<S>,
    name: impl Into<String>,
    arguments: Option<Map<String, Value>>,
) -> Result<PaidToolCallResult, McpClientError>
where
    C: McpToolCaller,
    S: PaymentSelector,
{
    X402McpClient::from_payment_client(caller, payments)
        .call_tool(name, arguments)
        .await
}
