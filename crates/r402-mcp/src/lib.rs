#![cfg_attr(docsrs, feature(doc_cfg))]
#![doc = include_str!("../README.md")]

//! MCP (Model Context Protocol) transport for x402.
//!
//! Built on the **official** Rust MCP SDK [`rmcp`](https://crates.io/crates/rmcp)
//! (`modelcontextprotocol/rust-sdk`), mirroring how Go binds
//! `modelcontextprotocol/go-sdk` and TypeScript binds
//! `@modelcontextprotocol/sdk`.
//!
//! # Feature flags
//!
//! | Feature | Surface |
//! |---------|---------|
//! | `server` | [`server::PaymentWrapper`] + encode helpers |
//! | `client` | [`client::X402McpClient`] auto-pay |
//! | `full` | server + client + telemetry |
//!
//! # Normative references
//!
//! - `specs/transports-v2/mcp.md`
//! - `go/mcp/{server,client,utils,types,constants}.go`
//! - `@x402/mcp` TypeScript package

pub mod constants;

#[cfg(feature = "telemetry")]
use tracing as _;

#[cfg(any(feature = "server", feature = "client"))]
#[cfg_attr(docsrs, doc(cfg(any(feature = "server", feature = "client"))))]
pub mod encode;

#[cfg(feature = "server")]
#[cfg_attr(docsrs, doc(cfg(feature = "server")))]
pub mod server;

#[cfg(feature = "client")]
#[cfg_attr(docsrs, doc(cfg(feature = "client")))]
pub mod client;

pub use constants::{
    MCP_PAYMENT_META_KEY, MCP_PAYMENT_REQUIRED_CODE, MCP_PAYMENT_RESPONSE_META_KEY,
    MCP_TOOL_URL_PREFIX,
};

#[cfg(any(feature = "server", feature = "client"))]
pub use encode::{
    McpPaymentPayload, attach_payment_to_params, attach_settle_response, create_tool_resource_url,
    extract_payment_from_params, extract_payment_required, extract_settle_response,
    is_payment_required_result, payment_required_tool_result, settlement_failed_tool_result,
};

#[cfg(feature = "server")]
pub use server::{PaymentWrapper, PaymentWrapperConfig, PaymentWrapperHooks, ServerHookContext};

#[cfg(feature = "client")]
pub use client::{
    McpClientError, McpToolCaller, PaidToolCallResult, PaymentSigner, X402McpClient,
    X402McpClientOptions,
};
