//! Protocol constants for MCP × x402.
//!
//! Values match foundation Go (`go/mcp/types.go`, `go/mcp/constants.go`) and
//! TypeScript (`@x402/mcp` `types/mcp.ts`).

/// JSON-RPC / MCP error code for payment required (x402).
///
/// Official value is the integer **402** (not a string).
pub const MCP_PAYMENT_REQUIRED_CODE: i32 = 402;

/// MCP `_meta` key for the client → server payment payload.
pub const MCP_PAYMENT_META_KEY: &str = "x402/payment";

/// MCP `_meta` key for the server → client settlement response.
pub const MCP_PAYMENT_RESPONSE_META_KEY: &str = "x402/payment-response";

/// Default tool resource URL prefix (`mcp://tool/{name}`).
pub const MCP_TOOL_URL_PREFIX: &str = "mcp://tool/";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn constants_match_foundation_go_and_ts() {
        assert_eq!(MCP_PAYMENT_REQUIRED_CODE, 402);
        assert_eq!(MCP_PAYMENT_META_KEY, "x402/payment");
        assert_eq!(MCP_PAYMENT_RESPONSE_META_KEY, "x402/payment-response");
    }
}
