//! Built-in integration with the official Rust MCP SDK ([`rmcp`] v0.15+).
//!
//! Enabled via the `rmcp` feature flag. Provides:
//!
//! - [`McpCaller`] implementation for [`rmcp::service::Peer<RoleClient>`]
//! - [`From`] conversions between `r402-mcp` and `rmcp` types
//! - [`PaymentWrapperRmcpExt`] for server-side direct rmcp type support
//!
//! # Client Example
//!
//! ```rust,ignore
//! use r402_mcp::client::X402McpClient;
//!
//! // rmcp Peer<RoleClient> implements McpCaller automatically
//! let session: rmcp::service::Peer<RoleClient> = /* ... */;
//! let client = X402McpClient::builder(session)
//!     .scheme_client(Box::new(evm_client))
//!     .build();
//!
//! let result = client.call_tool("paid_tool", args).await?;
//! ```
//!
//! # Server Example
//!
//! ```rust,ignore
//! use r402_mcp::rmcp_compat::PaymentWrapperRmcpExt;
//!
//! // In your rmcp ServerHandler
//! let rmcp_result = wrapper.process_rmcp(rmcp_params, |req| async {
//!     Ok(CallToolResult { content: vec![ContentItem::text("ok")], ..Default::default() })
//! }).await;
//! ```

use std::borrow::Cow;
use std::future::Future;

use r402::facilitator::BoxFuture;
use rmcp::model as mcp;
use rmcp::service::{Peer, RoleClient};

use crate::client::McpCaller;
use crate::error::McpPaymentError;
use crate::types::{CallToolParams, CallToolResult, ContentItem};

/// Converts rmcp [`Content`](mcp::Content) items to r402-mcp [`ContentItem`]s.
///
/// Only text content is extracted; non-text items (images, resources, etc.)
/// are silently skipped since the x402 payment protocol operates on text payloads.
#[must_use]
pub fn content_from_rmcp(content: &[mcp::Content]) -> Vec<ContentItem> {
    content
        .iter()
        .filter_map(|c| {
            let value = serde_json::to_value(c).ok()?;
            let type_str = value.get("type")?.as_str()?;
            if type_str == "text" {
                let text = value.get("text")?.as_str()?;
                Some(ContentItem::text(text))
            } else {
                None
            }
        })
        .collect()
}

/// Converts r402-mcp [`ContentItem`]s to rmcp [`Content`](mcp::Content) items.
///
/// # Panics
///
/// This function will not panic for well-formed `ContentItem::Text` values.
#[must_use]
pub fn content_to_rmcp(content: &[ContentItem]) -> Vec<mcp::Content> {
    content
        .iter()
        .filter_map(|item| {
            let ContentItem::Text { text } = item;
            let value = serde_json::json!({"type": "text", "text": text});
            serde_json::from_value(value).ok()
        })
        .collect()
}

/// Converts an rmcp [`CallToolResult`](mcp::CallToolResult) to r402-mcp's [`CallToolResult`].
#[must_use]
pub fn result_from_rmcp(result: &mcp::CallToolResult) -> CallToolResult {
    CallToolResult {
        content: content_from_rmcp(&result.content),
        is_error: result.is_error.unwrap_or(false),
        meta: result.meta.as_ref().map(|m| m.0.clone()),
        structured_content: result.structured_content.clone(),
    }
}

/// Converts r402-mcp's [`CallToolResult`] to rmcp's [`CallToolResult`](mcp::CallToolResult).
#[must_use]
pub fn result_to_rmcp(result: &CallToolResult) -> mcp::CallToolResult {
    mcp::CallToolResult {
        content: content_to_rmcp(&result.content),
        is_error: Some(result.is_error),
        meta: result.meta.as_ref().map(|m| mcp::Meta(m.clone())),
        structured_content: result.structured_content.clone(),
    }
}

impl From<mcp::CallToolRequestParams> for CallToolParams {
    fn from(params: mcp::CallToolRequestParams) -> Self {
        Self {
            name: params.name.into_owned(),
            arguments: params.arguments.unwrap_or_default(),
            meta: params.meta.map(|m| m.0),
        }
    }
}

impl From<CallToolParams> for mcp::CallToolRequestParams {
    fn from(params: CallToolParams) -> Self {
        Self {
            name: Cow::Owned(params.name),
            arguments: if params.arguments.is_empty() {
                None
            } else {
                Some(params.arguments)
            },
            meta: params.meta.map(mcp::Meta),
            task: None,
        }
    }
}

/// [`McpCaller`] implementation for rmcp's [`Peer<RoleClient>`].
///
/// This allows passing an rmcp client peer directly to
/// [`X402McpClient::builder()`](crate::client::X402McpClient::builder)
/// without any wrapper types.
impl McpCaller for Peer<RoleClient> {
    fn call_tool(
        &self,
        params: CallToolParams,
    ) -> BoxFuture<'_, Result<CallToolResult, McpPaymentError>> {
        Box::pin(async move {
            let rmcp_params: mcp::CallToolRequestParams = params.into();
            let rmcp_result = Self::call_tool(self, rmcp_params)
                .await
                .map_err(|e| McpPaymentError::ToolCallFailed(e.to_string()))?;
            Ok(result_from_rmcp(&rmcp_result))
        })
    }
}

/// Extension trait for [`PaymentWrapper`](crate::server::PaymentWrapper) providing
/// direct rmcp type support.
///
/// This eliminates manual type conversion in rmcp `ServerHandler` implementations.
pub trait PaymentWrapperRmcpExt {
    /// Processes an rmcp tool call with x402 payment enforcement.
    ///
    /// Accepts rmcp [`CallToolRequestParams`](mcp::CallToolRequestParams) directly
    /// and returns rmcp [`CallToolResult`](mcp::CallToolResult).
    ///
    /// The `handler` closure receives r402-mcp [`CallToolParams`] and should
    /// return r402-mcp [`CallToolResult`].
    fn process_rmcp<H, Fut>(
        &self,
        request: mcp::CallToolRequestParams,
        handler: H,
    ) -> impl Future<Output = mcp::CallToolResult> + Send
    where
        H: FnOnce(CallToolParams) -> Fut + Send,
        Fut: Future<Output = Result<CallToolResult, McpPaymentError>> + Send;
}

impl PaymentWrapperRmcpExt for crate::server::PaymentWrapper {
    #[allow(clippy::manual_async_fn)]
    fn process_rmcp<H, Fut>(
        &self,
        request: mcp::CallToolRequestParams,
        handler: H,
    ) -> impl Future<Output = mcp::CallToolResult> + Send
    where
        H: FnOnce(CallToolParams) -> Fut + Send,
        Fut: Future<Output = Result<CallToolResult, McpPaymentError>> + Send,
    {
        async move {
            let r402_params: CallToolParams = request.into();
            let r402_result = self.process(r402_params, handler).await;
            result_to_rmcp(&r402_result)
        }
    }
}
