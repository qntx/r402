# r402-mcp

Model Context Protocol (MCP) transport for the x402 payment protocol.

## Status

**Placeholder.** This crate reserves the `r402-mcp` name and documents the design
intent for an MCP transport. Concrete implementations (`server` and `client`
modules) will land in a subsequent release. Track progress in the repository's
issue tracker.

## Design Intent

The MCP transport will layer x402 on top of [Model Context Protocol tool calls]:

- **Server side**: wrap tool invocations with a paygate that emits a
  `PaymentRequired` payload in the tool's `_meta["x402/payment"]` block,
  producing an `isError: true` response until a valid payment is presented.
- **Client side**: a middleware that automatically signs and replays tool
  calls upon receiving a `PaymentRequired` response.

The wire-level types are shared with [`r402-core`](https://docs.rs/r402-core);
only the transport framing differs from the HTTP variant.

[Model Context Protocol tool calls]: https://modelcontextprotocol.io

## License

Licensed under either of MIT or Apache-2.0, at your option.
