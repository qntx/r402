# r402-mcp

Model Context Protocol (MCP) transport for the [x402 payment protocol][x402],
part of the [`r402`](../r402) workspace.

[x402]: https://www.x402.org

## Status

**Production-oriented skeleton** aligned with foundation Go (`go/mcp`) and
TypeScript (`@x402/mcp`) meta keys and flow:

| Surface | Feature | Notes |
| --- | --- | --- |
| Meta keys / error code | always | Match official `x402/payment`, `x402/payment-response`, `x402_payment_required` |
| [`server::PaymentWrapper`](src/server.rs) | `server` | Verify (+ optional settle) around a tool handler via any `Facilitator` |
| [`client::pay_and_call`](src/client.rs) | `client` | One automatic retry after payment-required |

Host applications supply their MCP SDK binding; r402 stays SDK-agnostic so
it does not pin a particular Rust MCP crate.

## Cargo features

| Feature | Surface |
| --- | --- |
| `server` | Payment wrapper for tool handlers |
| `client` | Auto-pay retry helper |
| `telemetry` | `tracing` hooks |
| `full` | All of the above |

## License

Dual-licensed under MIT and Apache-2.0.
