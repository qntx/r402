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

V1 payment paths are omitted (r402 is V2-only).

### Constants (must match foundation)

| Name | Value |
| ---- | ----- |
| `MCP_PAYMENT_REQUIRED_CODE` | **`402` (i32)** |
| `MCP_PAYMENT_META_KEY` | `"x402/payment"` |
| `MCP_PAYMENT_RESPONSE_META_KEY` | `"x402/payment-response"` |

Payment-required tool results set **both** `structuredContent` and
`content[0].text` (JSON of the same `PaymentRequired` object).

## Dependency

```toml
[dependencies]
r402-core = "0.14"
r402-mcp = { version = "0.15", features = ["server"] }   # or "client" / "full"
# optional umbrella:
# r402 = { version = "0.15", features = ["mcp", "server"] }
```

| Feature | Surface |
| ------- | ------- |
| `server` | `PaymentWrapper` + encode helpers |
| `client` | `X402McpClient` + `McpToolCaller` / `PaymentSigner` |
| `full` | server + client + telemetry |

## Server usage

Requires a [`Facilitator`](https://docs.rs/r402-core/latest/r402_core/trait.Facilitator.html)
(local chain crate or HTTP facilitator). Wire the wrapper around your tool
handler; this crate does **not** own the MCP transport loop — plug `invoke` /
`wrap` into whatever hosts `rmcp` (stdio, HTTP, etc.).

```rust,ignore
use std::sync::Arc;

use r402_core::{ResourceServer, wire::{PaymentRequirements, ResourceInfo}};
use r402_mcp::{PaymentWrapper, PaymentWrapperConfig};
use rmcp::model::{CallToolRequestParams, CallToolResult, ContentBlock};

// 1. Facilitator that can verify + settle (your implementation / HTTP client)
let facilitator = Arc::new(my_facilitator);
let resource_server = ResourceServer::new(facilitator);

// 2. Advertise one or more payment options for this tool
let accepts = vec![PaymentRequirements::new(
    "exact".into(),
    "eip155:84532".parse()?,
    "10000".into(),          // amount in asset smallest unit
    "0xPayTo...".into(),
    "0xUSDC...".into(),
    60,
)];
let config = PaymentWrapperConfig::try_new(
    accepts,
    Some(
        ResourceInfo::new("mcp://tool/get_weather")
            .with_description("Weather lookup")
            .with_mime_type("application/json"),
    ),
)?;
let wrapper = PaymentWrapper::try_new(resource_server, config)?;

// 3a. One-shot: wrap a single tools/call
async fn handle_tool(wrapper: &PaymentWrapper, params: CallToolRequestParams) -> CallToolResult {
    wrapper
        .invoke(params, |_params| async move {
            CallToolResult::success(vec![ContentBlock::text(r#"{"temp":18}"#)])
        })
        .await
}

// 3b. Cloneable handler for routers (Go Wrap equivalent)
let paid = wrapper.wrap(|_params| async move {
    CallToolResult::success(vec![ContentBlock::text("ok")])
});
// router.register("get_weather", paid);
```

Control flow (Go `PaymentWrapper.Wrap`):

1. Extract `_meta["x402/payment"]`
2. Match `accepts` → verify (tool-level 402 on failure)
3. Hooks → business handler
4. On tool success → settle → `_meta["x402/payment-response"]`
5. Settlement failure uses the **same dual-format** payment-required body (R5)

## Client usage

You implement two thin adapters:

- **`McpToolCaller`** — one method: call MCP `tools/call` (typically an
  `rmcp` client session).
- **`PaymentSigner`** — turn a `PaymentRequired` challenge into a signed
  `PaymentPayload` (scheme clients / your wallet glue).

```rust,ignore
use r402_core::wire::PaymentRequired;
use r402_mcp::{
    McpPaymentPayload, McpToolCaller, PaymentSigner, X402McpClient, X402McpClientOptions,
};
use rmcp::model::{CallToolRequestParams, CallToolResult};
use serde_json::Map;

struct MyMcpSession { /* rmcp Client / session */ }

impl McpToolCaller for MyMcpSession {
    async fn call_tool(
        &self,
        params: CallToolRequestParams,
    ) -> Result<CallToolResult, String> {
        // forward to rmcp session.call_tool(...)
        todo!()
    }
}

struct MySigner { /* scheme clients */ }

impl PaymentSigner for MySigner {
    async fn sign_payment(
        &self,
        required: PaymentRequired,
    ) -> Result<McpPaymentPayload, String> {
        // Select accepts[i], sign scheme payload → McpPaymentPayload::new(accepted, body)
        todo!()
    }
}

let client = X402McpClient::new(MyMcpSession { /* ... */ }, MySigner { /* ... */ })
    .with_options(X402McpClientOptions { auto_payment: true });

let out = client.call_tool("get_weather", Some(Map::from_iter([
    ("city".into(), serde_json::json!("SF")),
]))).await?;

assert!(out.payment_made);
// out.result — tool content; out.payment_response — settle meta if present
```

Free function: `call_paid_tool(caller, signer, name, args)`.

Hooks (`ClientHooks`): `on_payment_required` (abort / supply payload),
`on_payment_requested` (approve/deny), `on_before_payment`, `on_after_payment`.

## Encode helpers (shared)

| Helper | Role |
| ------ | ---- |
| `payment_required_tool_result` | dual-format 402 challenge |
| `settlement_failed_tool_result` | R5 dual-format settle failure |
| `extract_payment_required` | prefer structuredContent, else text |
| `attach_payment_to_params` / `extract_payment_from_params` | `_meta["x402/payment"]` |
| `attach_settle_response` / `extract_settle_response` | `_meta["x402/payment-response"]` |
| `create_tool_resource_url` | `mcp://tool/{name}` or custom |

## Known intentional gaps vs foundation

| Item | Notes |
| ---- | ----- |
| V1 | Not supported |
| SEP-1036 / JSON-RPC `-32042` | TS-only path; not in Go pin |
| Paid retry still 402 | Returns `McpClientError::StillRequired` (stricter than Go) |
| Live e2e example binary | Not shipped; use unit tests under `src/{server,client,encode}.rs` |

## License

Dual-licensed under MIT and Apache-2.0.
