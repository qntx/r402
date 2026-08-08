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

pub mod constants;

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

#[cfg(any(feature = "server", feature = "client"))]
pub use error::{McpClientError, PaymentRequiredError};

#[cfg(feature = "server")]
pub use server::{
    AfterExecutionContext, PaymentWrapper, PaymentWrapperConfig, PaymentWrapperConfigError,
    PaymentWrapperHooks, ServerHookContext, SettlementContext,
};

#[cfg(feature = "client")]
pub use client::{
    AfterPaymentContext, ClientHooks, McpToolCaller, PaidToolCallResult, PaymentRequiredContext,
    PaymentRequiredHookResult, PaymentSigner, X402McpClient, X402McpClientOptions, call_paid_tool,
};

// `serde` is a workspace dep used transitively via wire types; silence when
// no direct path reference exists under the selected feature set.
use serde as _;
