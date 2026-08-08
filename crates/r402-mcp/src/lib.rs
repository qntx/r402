#![cfg_attr(docsrs, feature(doc_cfg))]
#![doc = include_str!("../README.md")]

//! MCP (Model Context Protocol) transport for x402.
//!
//! This crate provides the **wire/meta contract** and payment wrapper helpers
//! aligned with the foundation Go (`go/mcp`) and TypeScript (`@x402/mcp`)
//! packages. Host applications plug their MCP SDK via the [`McpToolCaller`]
//! trait so r402 stays independent of a particular MCP Rust binding.
//!
//! # Status
//!
//! - **Meta keys / error codes**: stable, match official SDKs.
//! - **Server [`PaymentWrapper`]**: verify (+ optional settle) around a tool
//!   handler using any [`Facilitator`].
//! - **Client auto-pay loop**: [`pay_and_call`] retries once after a payment
//!   required error using a supplied payment signer callback.
//!
//! Feature flags: `server`, `client`, `telemetry` (see `Cargo.toml`).

use r402_core as _;

#[cfg(feature = "telemetry")]
use tracing as _;

/// Official meta key for payment-required payloads (tool result `_meta`).
pub const MCP_PAYMENT_META_KEY: &str = "x402/payment";

/// Official meta key for payment-response payloads after settlement.
pub const MCP_PAYMENT_RESPONSE_META_KEY: &str = "x402/payment-response";

/// Error code embedded when a tool call is rejected for missing payment.
pub const MCP_PAYMENT_REQUIRED_CODE: &str = "x402_payment_required";

#[cfg(feature = "server")]
#[cfg_attr(docsrs, doc(cfg(feature = "server")))]
pub mod server;

#[cfg(feature = "client")]
#[cfg_attr(docsrs, doc(cfg(feature = "client")))]
pub mod client;

#[cfg(test)]
mod tests {
    #[test]
    fn meta_keys_match_foundation() {
        // Go: constants.go / TS: types — keep these string-equal forever.
        assert_eq!(super::MCP_PAYMENT_META_KEY, "x402/payment");
        assert_eq!(super::MCP_PAYMENT_RESPONSE_META_KEY, "x402/payment-response");
        assert_eq!(super::MCP_PAYMENT_REQUIRED_CODE, "x402_payment_required");
    }
}
