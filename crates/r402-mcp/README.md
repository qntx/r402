# r402-mcp

MCP transport for the [x402 payment protocol][x402], part of the
[`r402`](../r402) workspace.

[x402]: https://www.x402.org

## Design

Binds the **official** Rust MCP SDK [`rmcp`](https://crates.io/crates/rmcp)
(`modelcontextprotocol/rust-sdk`), the same role as:

| Language | Official MCP SDK |
| -------- | ---------------- |
| Go | `github.com/modelcontextprotocol/go-sdk/mcp` |
| TypeScript | `@modelcontextprotocol/sdk` |
| **Rust (this crate)** | **`rmcp`** |

Protocol behaviour is pinned to:

- `specs/transports-v2/mcp.md`
- Go `go/mcp/*` (primary control-flow pin)
- TypeScript `@x402/mcp` (secondary)

### Constants (must match foundation)

| Name | Value |
| ---- | ----- |
| `MCP_PAYMENT_REQUIRED_CODE` | **`402` (i32)** |
| `MCP_PAYMENT_META_KEY` | `"x402/payment"` |
| `MCP_PAYMENT_RESPONSE_META_KEY` | `"x402/payment-response"` |

Payment-required tool results set **both** `structuredContent` and
`content[0].text` (JSON of the same `PaymentRequired` object).

## Features

| Feature | Surface |
| ------- | ------- |
| `server` | `PaymentWrapper` + encode helpers (`rmcp` model types) |
| `client` | `X402McpClient` + `McpToolCaller` / `PaymentSigner` traits |
| `full` | server + client + telemetry |

## Server (sketch)

```rust,ignore
use std::sync::Arc;
use r402_mcp::{PaymentWrapper, PaymentWrapperConfig};
use r402_core::wire::ResourceInfo;

let wrapper = PaymentWrapper::new(
    Arc::new(my_facilitator),
    PaymentWrapperConfig::new(accepts, ResourceInfo::new("mcp://tool/demo")),
);

// Inside your rmcp tool handler:
let result = wrapper.invoke(params, |params| async move {
    // business logic → CallToolResult::success(...)
}).await;
```

## Client (sketch)

```rust,ignore
use r402_mcp::{X402McpClient, McpToolCaller, PaymentSigner};

let client = X402McpClient::new(my_rmcp_peer_adapter, my_x402_signer);
let out = client.call_tool("demo", None).await?;
```

`PaymentSigner` is typically backed by `r402-http::X402Client` scheme
registration (wire payment without HTTP).

## License

Dual-licensed under MIT and Apache-2.0.
