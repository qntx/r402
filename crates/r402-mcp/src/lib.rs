#![cfg_attr(docsrs, feature(doc_cfg))]
#![doc = include_str!("../README.md")]
#![cfg_attr(
    test,
    allow(
        clippy::indexing_slicing,
        clippy::panic,
        clippy::expect_used,
        reason = "tests use panic/expect for clearer failure messages"
    )
)]

//! MCP transport for x402 on the official [`rmcp`] SDK.
//!
//! # Official pins
//!
//! | Component | Source |
//! |-----------|--------|
//! | Behaviour | `specs/transports-v2/mcp.md` |
//! | Control flow | Go `go/mcp/{server,client,utils}.go` |
//! | MCP types | **`rmcp`** (`modelcontextprotocol/rust-sdk`) |
//!
//! # Feature flags
//!
//! | Feature | Surface |
//! |---------|---------|
//! | `server` | [`PaymentWrapper`] |
//! | `client` | [`X402McpClient`], [`call_paid_tool`] |
//! | `full` | both + telemetry |

/// Legacy x402 JSON-RPC payment-required code (`402`). Client parse only.
pub const MCP_PAYMENT_REQUIRED_CODE: i32 = 402;
/// SEP-1036 `UrlElicitationRequired` JSON-RPC code (`-32042`). Client parse only.
pub const JSONRPC_PAYMENT_REQUIRED_CODE: i32 = -32042;
/// MCP `_meta` key for the client → server payment payload.
pub const MCP_PAYMENT_META_KEY: &str = "x402/payment";
/// MCP `_meta` key for the server → client settlement response.
pub const MCP_PAYMENT_RESPONSE_META_KEY: &str = "x402/payment-response";
/// Default tool resource URL prefix (`mcp://tool/{name}`).
pub const MCP_TOOL_URL_PREFIX: &str = "mcp://tool/";

#[cfg(test)]
mod constants_tests {
    use super::*;

    #[test]
    fn constants_match_foundation_go_and_ts() {
        assert_eq!(MCP_PAYMENT_REQUIRED_CODE, 402);
        assert_eq!(JSONRPC_PAYMENT_REQUIRED_CODE, -32042);
        assert_eq!(MCP_PAYMENT_META_KEY, "x402/payment");
        assert_eq!(MCP_PAYMENT_RESPONSE_META_KEY, "x402/payment-response");
    }
}

#[cfg(feature = "telemetry")]
use tracing as _;

#[cfg(any(feature = "server", feature = "client"))]
#[cfg_attr(docsrs, doc(cfg(any(feature = "server", feature = "client"))))]
pub mod encode;

#[cfg(any(feature = "server", feature = "client"))]
#[cfg_attr(docsrs, doc(cfg(any(feature = "server", feature = "client"))))]
pub mod error;

#[cfg(feature = "server")]
#[cfg_attr(docsrs, doc(cfg(feature = "server")))]
pub mod server;

#[cfg(feature = "client")]
#[cfg_attr(docsrs, doc(cfg(feature = "client")))]
pub mod client;

#[cfg(feature = "client")]
pub use client::{
    AfterPaymentContext, McpClientHooks, McpToolCaller, PaidToolCallResult, PaymentRequiredContext,
    PaymentRequiredHookResult, PaymentSigner, X402McpClient, X402McpClientOptions, call_paid_tool,
};
#[cfg(any(feature = "server", feature = "client"))]
pub use encode::{
    McpPaymentPayload, attach_payment_to_params, attach_settle_response, create_tool_resource_url,
    extract_payment_from_params, extract_payment_required, extract_payment_required_from_rpc,
    extract_settle_response, is_payment_required_result, is_payment_required_rpc,
    payment_required_tool_result, settlement_failed_tool_result,
};
#[cfg(feature = "client")]
pub use error::mcp_call_error_from_rmcp;
#[cfg(any(feature = "server", feature = "client"))]
pub use error::{McpCallError, McpClientError, PaymentRequiredError};
// `serde` is a workspace dep used transitively via wire types; silence when
// no direct path reference exists under the selected feature set.
use serde as _;
#[cfg(feature = "server")]
pub use server::{
    AfterExecutionContext, PaymentWrapper, PaymentWrapperConfig, PaymentWrapperConfigError,
    PaymentWrapperHooks, ServerHookContext, SettlementContext,
};
