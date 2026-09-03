//! MCP transport for the x402 payment protocol (official [`rmcp`] SDK).
//!
//! `_meta` keys live in [`meta`]. MCP v1 does not expose [`r402_server::SettlementMode`].

#![cfg_attr(docsrs, feature(doc_cfg))]
#![cfg_attr(
    test,
    allow(
        clippy::indexing_slicing,
        clippy::panic,
        clippy::expect_used,
        clippy::unwrap_used,
        reason = "tests use panic/expect for clearer failure messages"
    )
)]

mod error;
mod meta;

#[cfg(feature = "client")]
pub use error::mcp_call_error_from_rmcp;
pub use error::{McpCallError, McpClientError, PaymentRequiredError};
pub use meta::{
    JSONRPC_PAYMENT_REQUIRED_CODE, MCP_PAYMENT_META_KEY, MCP_PAYMENT_REQUIRED_CODE,
    MCP_PAYMENT_RESPONSE_META_KEY, MCP_TOOL_URL_PREFIX,
};

#[cfg(any(feature = "client", feature = "server"))]
mod tool;
#[cfg(any(feature = "client", feature = "server"))]
pub use tool::{
    McpPaymentPayload, attach_payment_to_params, attach_settle_response, create_tool_resource_url,
    extract_payment_from_params, extract_payment_required, extract_payment_required_from_rpc,
    extract_settle_response, is_payment_required_result, is_payment_required_rpc,
    payment_payload_from_signed, payment_required_tool_result, settlement_failed_tool_result,
};

#[cfg(feature = "client")]
mod buyer;
#[cfg(feature = "client")]
pub use buyer::{
    AfterPaymentContext, McpClientHooks, McpToolCaller, PaidToolCallResult, PaymentRequiredContext,
    PaymentRequiredHookResult, X402McpClient, X402McpClientOptions, call_paid_tool,
};

#[cfg(feature = "server")]
mod wrapper;
#[cfg(test)]
use tokio as _;
#[cfg(feature = "telemetry")]
use tracing as _;
#[cfg(feature = "server")]
pub use wrapper::{
    AfterExecutionContext, PaymentWrapper, PaymentWrapperConfig, PaymentWrapperConfigError,
    PaymentWrapperHooks, ServerHookContext, SettlementContext,
};
