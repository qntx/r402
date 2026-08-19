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
| `near` | [`r402-near`](https://docs.rs/r402-near) | NEAR exact (NEP-141 / NEP-366) |
| `xrpl` | [`r402-xrpl`](https://docs.rs/r402-xrpl) | XRPL exact (XRP / RLUSD) |
| `hedera` | [`r402-hedera`](https://docs.rs/r402-hedera) | Hedera exact (HBAR / HTS) |
| `algorand` | [`r402-algorand`](https://docs.rs/r402-algorand) | Algorand exact (ASA / algod REST) |
| `aptos` | [`r402-aptos`](https://docs.rs/r402-aptos) | Aptos exact (FA / aptos-sdk 0.6) |
| `keeta` | [`r402-keeta`](https://docs.rs/r402-keeta) | Keeta exact (signed `SEND` block) |
| `tvm` | [`r402-tvm`](https://docs.rs/r402-tvm) | TON exact (TEP-74 / W5R1) |
| `stellar` | [`r402-stellar`](https://docs.rs/r402-stellar) | Stellar exact (SEP-41 auth entries) |
| `http` (default) | [`r402-http`](https://docs.rs/r402-http) | Axum middleware + reqwest client |
| `mcp` | [`r402-mcp`](https://docs.rs/r402-mcp) | MCP transport on official `rmcp` (V2) |

Forwarding features: `client`, `server`, `facilitator`, `telemetry`, `cache`,
`ext-bazaar`, `ext-payment-id`, `all-extensions`, `full`.

## Quick Start

```toml
[dependencies]
# Minimal EVM HTTP server/client
r402 = { version = "0.16", features = ["evm", "http", "client", "server", "facilitator"] }

# Multi-chain
r402 = { version = "0.16", features = [
  "evm", "solana", "tron", "casper", "near", "tvm", "xrpl",
  "hedera", "algorand", "keeta", "stellar",
  "http", "client", "server", "facilitator",
] }
```

```rust,ignore
// After enabling the matching features:
use r402::evm;    // r402-evm
use r402::solana; // r402-solana
use r402::tron;   // r402-tron
use r402::casper; // r402-casper
use r402::near;   // r402-near
use r402::xrpl;   // r402-xrpl
use r402::hedera; // r402-hedera
use r402::algorand; // r402-algorand
use r402::aptos;    // r402-aptos
use r402::keeta;  // r402-keeta
use r402::tvm;    // r402-tvm
use r402::stellar; // r402-stellar
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
| `near` | | Re-export `r402-near` as [`near`] |
| `xrpl` | | Re-export `r402-xrpl` as [`xrpl`] |
| `hedera` | | Re-export `r402-hedera` as [`hedera`] |
| `algorand` | | Re-export `r402-algorand` as [`algorand`] |
| `tvm` | | Re-export `r402-tvm` as [`tvm`] |
| `keeta` | | Re-export `r402-keeta` as [`keeta`] |
| `stellar` | | Re-export `r402-stellar` as [`stellar`] |
| `aptos` | | Re-export `r402-aptos` as [`aptos`] |
| `keeta` | | Re-export `r402-keeta` as [`keeta`] |
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
