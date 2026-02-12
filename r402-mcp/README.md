# r402-mcp

MCP (Model Context Protocol) integration for the x402 payment protocol.

This crate enables paid tool calls in MCP servers and automatic payment handling in MCP clients, following the [x402 payment protocol](https://github.com/coinbase/x402) specification.

## Features

- **Client-side auto-payment** — `X402McpClient` wraps any MCP caller with automatic 402 payment handling
- **Server-side payment wrapper** — `PaymentWrapper` wraps tool handlers with verify → execute → settle lifecycle
- **Framework-agnostic** — Works with any MCP SDK via the `McpCaller` trait and `serde_json::Value`-based types
- **Lifecycle hooks** — Trait-based hooks for both client and server sides
- **Payment extraction utilities** — Extract/attach payment data from/to MCP `_meta` fields

## Architecture

The crate follows the x402 MCP specification where payment data flows through JSON-RPC `_meta` fields:

- `_meta["x402/payment"]` — Client → Server: payment payload
- `_meta["x402/payment-response"]` — Server → Client: settlement response

### Client Flow

1. Call tool without payment
2. If 402: extract `PaymentRequired` from `structuredContent` or `content[0].text`
3. Create payment via registered `SchemeClient`s
4. Retry with payment in `_meta["x402/payment"]`
5. Extract settlement response from result `_meta`

### Server Flow

1. Extract `x402/payment` from request `_meta`
2. If missing → return 402 error with `PaymentRequired`
3. Verify payment via `Facilitator`
4. Execute original handler
5. Settle payment via `Facilitator`
6. Attach `SettleResponse` to result `_meta`

## Usage

### Client

```rust,ignore
use r402_mcp::client::{X402McpClient, McpCaller};
use r402_mcp::types::ClientOptions;

let client = X402McpClient::builder(my_mcp_session)
    .scheme_client(Box::new(evm_scheme_client))
    .auto_payment(true)
    .build();

let result = client.call_tool("get_weather", args).await?;
if let Some(settle) = &result.payment_response {
    println!("Paid! tx: {:?}", settle);
}
```

### Server

```rust,ignore
use r402_mcp::server::PaymentWrapper;
use r402_mcp::types::PaymentWrapperConfig;

let wrapper = PaymentWrapper::new(facilitator.into(), PaymentWrapperConfig {
    accepts: vec![payment_requirements],
    resource: Some(resource_info),
    ..Default::default()
});

let result = wrapper.process(request, |req| async {
    Ok(CallToolResult {
        content: vec![ContentItem::text("Weather: sunny")],
        ..Default::default()
    })
}).await;
```

### Low-level Utilities

```rust
use r402_mcp::extract;

// Extract payment from meta
let payment = extract::extract_payment_from_meta(&meta);

// Attach payment to meta
extract::attach_payment_to_meta(&mut meta, payment_value);

// Extract payment required from error result
let pr = extract::extract_payment_required_from_result(&result);
```

## Feature Flags

| Feature     | Description                                                               |
| ----------- | ------------------------------------------------------------------------- |
| `rmcp`      | Built-in integration with the official [`rmcp`](https://docs.rs/rmcp) SDK |
| `telemetry` | Enables `tracing` instrumentation                                         |

## License

Licensed under either of [Apache License, Version 2.0](../LICENSE-APACHE) or [MIT license](../LICENSE-MIT) at your option.
