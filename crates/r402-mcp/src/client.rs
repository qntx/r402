//! Client-side MCP auto-pay (Go `X402MCPClient` / `CallPaidTool` shape).
//!
//! Hosts supply an [`McpToolCaller`] (satisfied by tests or an `rmcp` peer
//! adapter). Payment signing is a callback so `r402-http::X402Client` can be
//! wired without a hard dependency cycle.

use std::future::Future;

use r402_core::wire::{PaymentRequired, SettleResponse};
use rmcp::model::{CallToolRequestParams, CallToolResult};
use serde_json::Value;

use crate::encode::{
    McpPaymentPayload, attach_payment_to_params, extract_payment_required, extract_settle_response,
    is_payment_required_result,
};

/// Errors from paid tool calls.
#[derive(Debug, thiserror::Error)]
pub enum McpClientError {
    /// Underlying MCP transport / tool call failed.
    #[error("mcp transport: {0}")]
    Transport(String),
    /// Building a payment payload failed.
    #[error("payment creation: {0}")]
    Payment(String),
    /// Server still required payment after a signed retry.
    #[error("payment still required after retry")]
    StillRequired,
    /// Auto-payment disabled and payment was required.
    #[error("payment required but auto_payment is disabled")]
    AutoPaymentDisabled,
}

/// Minimal tool-call surface (Go `MCPCaller`).
pub trait McpToolCaller: Send + Sync {
    /// Invokes `tools/call` with the given params.
    fn call_tool(
        &self,
        params: CallToolRequestParams,
    ) -> impl Future<Output = Result<CallToolResult, String>> + Send;
}

/// Result of a paid tool call.
#[derive(Debug, Clone)]
pub struct PaidToolCallResult {
    /// Underlying MCP tool result.
    pub result: CallToolResult,
    /// Whether a payment was submitted on the retry path.
    pub payment_made: bool,
    /// Settlement response from `_meta`, if present.
    pub payment_response: Option<SettleResponse>,
}

/// Options for [`X402McpClient`].
#[derive(Debug, Clone, Copy)]
pub struct X402McpClientOptions {
    /// When true (default), automatically sign and retry on payment-required.
    pub auto_payment: bool,
}

impl Default for X402McpClientOptions {
    fn default() -> Self {
        Self {
            auto_payment: true,
        }
    }
}

/// Signs a [`PaymentRequired`] into a wire [`McpPaymentPayload`].
pub trait PaymentSigner: Send + Sync {
    /// Creates a payment payload for the given requirements envelope.
    fn sign_payment(
        &self,
        required: PaymentRequired,
    ) -> impl Future<Output = Result<McpPaymentPayload, String>> + Send;
}

/// x402-aware MCP client wrapper.
pub struct X402McpClient<C, S> {
    caller: C,
    signer: S,
    options: X402McpClientOptions,
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
    /// Creates a client with default options (`auto_payment: true`).
    #[must_use]
    pub const fn new(caller: C, signer: S) -> Self {
        Self {
            caller,
            signer,
            options: X402McpClientOptions {
                auto_payment: true,
            },
        }
    }

    /// Sets client options.
    #[must_use]
    pub const fn with_options(mut self, options: X402McpClientOptions) -> Self {
        self.options = options;
        self
    }

    /// Calls a tool, automatically paying when required.
    ///
    /// # Errors
    ///
    /// Returns [`McpClientError`] on transport, signing, or persistent 402.
    pub async fn call_tool(
        &self,
        name: impl Into<String>,
        arguments: Option<Value>,
    ) -> Result<PaidToolCallResult, McpClientError> {
        let name = name.into();
        let mut params = CallToolRequestParams::new(name.clone());
        if let Some(Value::Object(map)) = arguments {
            params = params.with_arguments(map);
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

        if !self.options.auto_payment {
            return Err(McpClientError::AutoPaymentDisabled);
        }

        let required = extract_payment_required(&first)
            .ok_or_else(|| McpClientError::Payment("missing PaymentRequired body".into()))?;

        let payload = self
            .signer
            .sign_payment(required)
            .await
            .map_err(McpClientError::Payment)?;

        let paid_params = attach_payment_to_params(params, &payload);
        let second = self
            .caller
            .call_tool(paid_params)
            .await
            .map_err(McpClientError::Transport)?;

        if is_payment_required_result(&second) {
            return Err(McpClientError::StillRequired);
        }

        Ok(PaidToolCallResult {
            payment_response: extract_settle_response(&second),
            result: second,
            payment_made: true,
        })
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use r402_core::wire::{PaymentRequired, PaymentRequirements, ResourceInfo};
    use serde_json::json;

    use super::*;
    use crate::encode::payment_required_tool_result;

    struct MockCaller {
        calls: Mutex<u8>,
    }

    impl McpToolCaller for MockCaller {
        async fn call_tool(
            &self,
            params: CallToolRequestParams,
        ) -> Result<CallToolResult, String> {
            let unpaid = params.meta.is_none();
            {
                let mut n = self.calls.lock().map_err(|e| e.to_string())?;
                *n = n.saturating_add(1);
            }
            if unpaid {
                let resource = ResourceInfo::new("mcp://tool/demo");
                let req = PaymentRequirements::new(
                    "exact".into(),
                    "eip155:1".parse().unwrap(),
                    "1".into(),
                    "0xa".into(),
                    "0xb".into(),
                    60,
                );
                let pr = PaymentRequired::new(resource).with_accepts(vec![req]);
                return Ok(payment_required_tool_result(&pr));
            }
            Ok(CallToolResult::success(vec![rmcp::model::ContentBlock::text(
                "ok",
            )]))
        }
    }

    struct MockSigner;

    impl PaymentSigner for MockSigner {
        async fn sign_payment(
            &self,
            required: PaymentRequired,
        ) -> Result<McpPaymentPayload, String> {
            let accepted = required
                .accepts
                .into_iter()
                .next()
                .ok_or_else(|| "no accepts".to_owned())?;
            Ok(McpPaymentPayload::new(accepted, json!({})))
        }
    }

    #[tokio::test]
    async fn auto_pays_on_second_call() {
        let client = X402McpClient::new(MockCaller { calls: Mutex::new(0) }, MockSigner);
        let out = client.call_tool("demo", None).await.unwrap();
        assert!(out.payment_made);
        assert!(!out.result.is_error.unwrap_or(false));
    }
}
