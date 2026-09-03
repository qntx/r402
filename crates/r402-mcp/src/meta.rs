//! MCP `_meta` keys and JSON-RPC payment-required codes.
//!
//! Server never emits JSON-RPC `402` / `-32042`; the client may parse them.

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
