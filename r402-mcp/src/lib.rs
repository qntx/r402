#![cfg_attr(docsrs, feature(doc_cfg))]

//! MCP (Model Context Protocol) integration for the x402 payment protocol.
//!
//! This crate enables paid tool calls in MCP servers and automatic payment
//! handling in MCP clients, following the x402 payment protocol specification.
//!
//! # Architecture
//!
//! The crate provides framework-agnostic types and utilities that work with
//! any MCP SDK implementation via [`serde_json::Value`]-based interfaces.
//!
//! # Client Usage
//!
//! Wrap an MCP session with automatic x402 payment handling:
//!
//! ```rust,ignore
//! use r402_mcp::client::{X402McpClient, ClientOptions};
//!
//! let mcp_client = X402McpClient::builder(my_mcp_caller)
//!     .scheme_client(evm_scheme_client)
//!     .build();
//!
//! // Tool calls automatically handle 402 payment flows
//! let result = mcp_client.call_tool("get_weather", args).await?;
//! ```
//!
//! # Server Usage
//!
//! Wrap tool handlers with payment verification and settlement:
//!
//! ```rust,ignore
//! use r402_mcp::server::{PaymentWrapper, PaymentWrapperConfig};
//!
//! let wrapper = PaymentWrapper::new(facilitator, PaymentWrapperConfig {
//!     accepts: payment_requirements,
//!     resource: Some(resource_info),
//!     ..Default::default()
//! });
//!
//! // Process tool calls with automatic payment enforcement
//! let result = wrapper.process(request, |req| async { handle_tool(req).await }).await;
//! ```
//!
//! # Utility Functions
//!
//! The [`extract`] module provides low-level helpers for working with
//! x402 payment data in MCP `_meta` fields:
//!
//! - [`extract::extract_payment_from_meta`] - Extract payment payload from request meta
//! - [`extract::attach_payment_to_meta`] - Attach payment payload to request meta
//! - [`extract::extract_payment_response_from_meta`] - Extract settlement response from result meta
//! - [`extract::extract_payment_required_from_result`] - Extract 402 info from error results
//!
//! # Feature Flags
//!
//! - `rmcp` — Built-in integration with the official [`rmcp`](https://docs.rs/rmcp) Rust MCP SDK
//! - `telemetry` — Enables tracing instrumentation for debugging and monitoring

pub mod client;
pub mod error;
pub mod extract;
pub mod server;
pub mod types;

#[cfg(feature = "rmcp")]
#[cfg_attr(docsrs, doc(cfg(feature = "rmcp")))]
pub mod rmcp_compat;

/// MCP `_meta` key for sending payment payloads (client → server).
pub const PAYMENT_META_KEY: &str = "x402/payment";

/// MCP `_meta` key for settlement responses (server → client).
pub const PAYMENT_RESPONSE_META_KEY: &str = "x402/payment-response";

/// JSON-RPC error code for payment required (x402).
pub const PAYMENT_REQUIRED_CODE: i32 = 402;

/// MCP error envelope key for x402 payment errors (TS SDK compatibility).
///
/// The `@x402/mcp` TS SDK wraps 402 errors as:
/// ```json
/// { "x402/error": { "code": 402, "data": { /* PaymentRequired */ } } }
/// ```
pub const PAYMENT_ERROR_KEY: &str = "x402/error";
