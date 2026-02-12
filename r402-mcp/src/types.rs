//! Protocol types for MCP x402 payment integration.
//!
//! This module defines the framework-agnostic types used throughout the
//! MCP payment flow, including tool call parameters, results, hook contexts,
//! and configuration structures.

use std::collections::HashMap;

use r402::facilitator::BoxFuture;
use r402::proto;
use serde::{Deserialize, Serialize};

use crate::error::McpPaymentError;

/// Parameters for calling an MCP tool.
///
/// This is a framework-agnostic representation of MCP `CallToolParams`.
/// The `meta` field carries the x402 `_meta` data for payment flows.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CallToolParams {
    /// The tool name to invoke.
    pub name: String,
    /// Arguments to pass to the tool.
    #[serde(default)]
    pub arguments: serde_json::Map<String, serde_json::Value>,
    /// Optional `_meta` field for protocol extensions (x402 payment data).
    #[serde(rename = "_meta", default, skip_serializing_if = "Option::is_none")]
    pub meta: Option<serde_json::Map<String, serde_json::Value>>,
}

/// A single content item in a tool call result.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
#[non_exhaustive]
pub enum ContentItem {
    /// Text content.
    Text {
        /// The text value.
        text: String,
    },
}

impl ContentItem {
    /// Creates a new text content item.
    #[must_use]
    pub fn text(text: impl Into<String>) -> Self {
        Self::Text { text: text.into() }
    }

    /// Returns the text content if this is a text item.
    #[must_use]
    pub fn as_text(&self) -> Option<&str> {
        match self {
            Self::Text { text } => Some(text),
        }
    }
}

/// Result of an MCP tool call.
///
/// This is a framework-agnostic representation of MCP `CallToolResult`.
/// The `meta` field may contain x402 settlement responses.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CallToolResult {
    /// Content items returned by the tool.
    #[serde(default)]
    pub content: Vec<ContentItem>,
    /// Whether the tool returned an error.
    #[serde(default, rename = "isError")]
    pub is_error: bool,
    /// Optional `_meta` field for protocol extensions.
    #[serde(rename = "_meta", default, skip_serializing_if = "Option::is_none")]
    pub meta: Option<serde_json::Map<String, serde_json::Value>>,
    /// Optional structured content (preferred for payment required responses).
    #[serde(
        default,
        rename = "structuredContent",
        skip_serializing_if = "Option::is_none"
    )]
    pub structured_content: Option<serde_json::Value>,
}

/// Result of a paid MCP tool call, with payment metadata.
#[derive(Debug, Clone)]
pub struct PaidToolCallResult {
    /// Content items from the tool response.
    pub content: Vec<ContentItem>,
    /// Whether the tool returned an error.
    pub is_error: bool,
    /// The settlement response, if payment was made.
    pub payment_response: Option<proto::SettleResponse>,
    /// Whether a payment was made during this call.
    pub payment_made: bool,
    /// The raw tool call result.
    pub raw_result: CallToolResult,
}

/// Context for a tool call, provided to hooks.
#[derive(Debug, Clone)]
pub struct ToolCallContext {
    /// The tool name being called.
    pub tool_name: String,
    /// The arguments passed to the tool.
    pub arguments: serde_json::Map<String, serde_json::Value>,
    /// The `_meta` field from the request.
    pub meta: Option<serde_json::Map<String, serde_json::Value>>,
}

/// Context provided to client-side payment required hooks.
#[derive(Debug, Clone)]
pub struct PaymentRequiredContext {
    /// The tool name that requires payment.
    pub tool_name: String,
    /// The arguments passed to the tool.
    pub arguments: serde_json::Map<String, serde_json::Value>,
    /// The payment required response from the server.
    pub payment_required: proto::PaymentRequired,
}

/// Context provided to client-side before-payment hooks.
#[derive(Debug, Clone)]
pub struct BeforePaymentContext {
    /// The tool name that requires payment.
    pub tool_name: String,
    /// The payment requirements from the server.
    pub payment_required: proto::PaymentRequired,
}

/// Context provided to client-side after-payment hooks.
#[derive(Debug, Clone)]
pub struct AfterPaymentContext {
    /// The tool name that was paid for.
    pub tool_name: String,
    /// The payment payload that was sent.
    pub payment_payload: serde_json::Value,
    /// The tool call result.
    pub result: CallToolResult,
    /// The settlement response, if available.
    pub settle_response: Option<proto::SettleResponse>,
}

/// Client-side options for the x402 MCP client.
#[derive(Debug, Clone, Copy)]
pub struct ClientOptions {
    /// Whether to automatically handle payments when a tool requires them.
    /// Defaults to `true`.
    pub auto_payment: bool,
}

impl Default for ClientOptions {
    fn default() -> Self {
        Self { auto_payment: true }
    }
}

/// Trait for client-side MCP payment lifecycle hooks.
///
/// All methods have default no-op implementations. Override only the
/// hooks you need.
pub trait ClientHooks: Send + Sync {
    /// Called when a tool returns a 402 payment required response.
    ///
    /// Return `Some(payload)` to use a custom payment, `None` to proceed
    /// with automatic payment creation. Return an error to abort.
    fn on_payment_required(
        &self,
        _ctx: &PaymentRequiredContext,
    ) -> BoxFuture<'_, Result<Option<serde_json::Value>, McpPaymentError>> {
        Box::pin(async { Ok(None) })
    }

    /// Called before automatic payment creation.
    ///
    /// Return `true` to approve, `false` to deny (triggers [`McpPaymentError::Aborted`]).
    fn on_payment_requested(
        &self,
        _ctx: &PaymentRequiredContext,
    ) -> BoxFuture<'_, Result<bool, McpPaymentError>> {
        Box::pin(async { Ok(true) })
    }

    /// Called before a payment payload is created and signed.
    ///
    /// Use this for logging, metrics, or pre-flight checks.
    fn on_before_payment(
        &self,
        _ctx: &BeforePaymentContext,
    ) -> BoxFuture<'_, Result<(), McpPaymentError>> {
        Box::pin(async { Ok(()) })
    }

    /// Called after a payment is submitted and the tool returns a result.
    fn on_after_payment(
        &self,
        _ctx: &AfterPaymentContext,
    ) -> BoxFuture<'_, Result<(), McpPaymentError>> {
        Box::pin(async { Ok(()) })
    }
}

/// No-op implementation of [`ClientHooks`] for when no hooks are needed.
#[derive(Debug, Clone, Copy)]
pub struct NoClientHooks;

impl ClientHooks for NoClientHooks {}

/// Context provided to server-side hooks during payment processing.
#[derive(Debug, Clone)]
pub struct ServerHookContext {
    /// The tool name being executed.
    pub tool_name: String,
    /// The arguments passed to the tool.
    pub arguments: serde_json::Map<String, serde_json::Value>,
    /// The payment requirements matched.
    pub payment_requirements: proto::v2::PaymentRequirements,
    /// The payment payload from the client.
    pub payment_payload: serde_json::Value,
}

/// Context provided to after-execution hooks.
#[derive(Debug, Clone)]
pub struct AfterExecutionContext {
    /// The server hook context.
    pub server_ctx: ServerHookContext,
    /// The tool call result.
    pub result: CallToolResult,
}

/// Context provided to after-settlement hooks.
#[derive(Debug, Clone)]
pub struct SettlementContext {
    /// The server hook context.
    pub server_ctx: ServerHookContext,
    /// The settlement response.
    pub settlement: proto::SettleResponse,
}

/// Trait for server-side MCP payment lifecycle hooks.
///
/// All methods have default no-op implementations. Override only the
/// hooks you need.
pub trait ServerHooks: Send + Sync {
    /// Called before tool execution, after payment verification.
    ///
    /// Return `true` to proceed, `false` to abort execution.
    fn on_before_execution(
        &self,
        _ctx: &ServerHookContext,
    ) -> BoxFuture<'_, Result<bool, McpPaymentError>> {
        Box::pin(async { Ok(true) })
    }

    /// Called after tool execution, before settlement.
    fn on_after_execution(
        &self,
        _ctx: &AfterExecutionContext,
    ) -> BoxFuture<'_, Result<(), McpPaymentError>> {
        Box::pin(async { Ok(()) })
    }

    /// Called after successful settlement.
    fn on_after_settlement(
        &self,
        _ctx: &SettlementContext,
    ) -> BoxFuture<'_, Result<(), McpPaymentError>> {
        Box::pin(async { Ok(()) })
    }
}

/// No-op implementation of [`ServerHooks`] for when no hooks are needed.
#[derive(Debug, Clone, Copy)]
pub struct NoServerHooks;

impl ServerHooks for NoServerHooks {}

/// Configuration for the server-side [`PaymentWrapper`](crate::server::PaymentWrapper).
pub struct PaymentWrapperConfig {
    /// Acceptable payment methods for the wrapped tool.
    pub accepts: Vec<proto::v2::PaymentRequirements>,
    /// Optional resource metadata.
    pub resource: Option<proto::v2::ResourceInfo>,
    /// Optional server-side hooks.
    pub hooks: Option<Box<dyn ServerHooks>>,
    /// Optional protocol extensions.
    pub extensions: Option<HashMap<String, serde_json::Value>>,
}

#[allow(clippy::derivable_impls)]
impl Default for PaymentWrapperConfig {
    fn default() -> Self {
        Self {
            accepts: Vec::new(),
            resource: None,
            hooks: None,
            extensions: None,
        }
    }
}

impl std::fmt::Debug for PaymentWrapperConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PaymentWrapperConfig")
            .field("accepts", &self.accepts)
            .field("resource", &self.resource)
            .field("hooks", &self.hooks.as_ref().map(|_| "<dyn ServerHooks>"))
            .field("extensions", &self.extensions)
            .finish()
    }
}
