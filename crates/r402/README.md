# r402

x402 Payment Protocol SDK for Rust — **umbrella crate**.

This crate re-exports the workspace under feature flags. Prefer depending on
individual crates when you need a minimal dependency graph.

## Included crates

| Feature | Crate | Role |
| --- | --- | --- |
| *(always)* | [`r402-core`](https://docs.rs/r402-core) | Wire types, `Facilitator`, schemes, hooks, extensions |
| `evm` (default) | [`r402-evm`](https://docs.rs/r402-evm) | EVM exact + upto (ERC-3009 / Permit2) |
| `solana` | [`r402-solana`](https://docs.rs/r402-solana) | Solana exact (SPL Token / Token-2022) |
| `tron` | [`r402-tron`](https://docs.rs/r402-tron) | Tron exact (TIP-712 / EIP-3009 + Permit2) |
| `casper` | [`r402-casper`](https://docs.rs/r402-casper) | Casper exact (CEP-18, remote facilitator) |
| `http` (default) | [`r402-http`](https://docs.rs/r402-http) | Axum middleware + reqwest client |
| `mcp` | [`r402-mcp`](https://docs.rs/r402-mcp) | MCP transport on official `rmcp` (V2) |

Forwarding features: `client`, `server`, `facilitator`, `telemetry`, `cache`,
`ext-bazaar`, `ext-payment-id`, `all-extensions`, `full`.

## Quick Start

```toml
[dependencies]
# Minimal EVM HTTP server/client
r402 = { version = "0.15", features = ["evm", "http", "client", "server", "facilitator"] }

# Multi-chain
r402 = { version = "0.15", features = [
  "evm", "solana", "tron", "casper",
  "http", "client", "server", "facilitator",
] }
```

```rust,ignore
// After enabling the matching features:
use r402::evm;    // r402-evm
use r402::solana; // r402-solana
use r402::tron;   // r402-tron
use r402::casper; // r402-casper
use r402::http;   // r402-http
use r402::mcp;    // r402-mcp (needs feature "mcp")
```

## Features

| Flag | Default | What it enables |
| --- | :---: | --- |
| `evm` | yes | Re-export `r402-evm` as [`evm`] |
| `solana` | | Re-export `r402-solana` as [`solana`] |
| `tron` | | Re-export `r402-tron` as [`tron`] |
| `casper` | | Re-export `r402-casper` as [`casper`] |
| `http` | yes | Re-export `r402-http` as [`http`] |
| `mcp` | | Re-export `r402-mcp` as [`mcp`] |
| `client` | | Forward `client` to enabled chain + http crates |
| `server` | | Forward `server` to enabled chain + http crates |
| `facilitator` | | Forward `facilitator` (Casper also enables `http-client`) |
| `telemetry` | | Forward `telemetry` / `tracing` |
| `cache` | | `r402-core` settlement cache |
| `ext-bazaar` | | Bazaar extension |
| `ext-payment-id` | | Payment-identifier extension |
| `all-extensions` | | All built-in extensions |
| `full` | | Every feature above |

## License

Licensed under either of MIT or Apache-2.0, at your option.
