# r402

x402 Payment Protocol SDK for Rust — umbrella crate.

This crate is a convenience re-export of the component crates that make up the r402 SDK.
For finer-grained control over compile-time and runtime dependencies, depend on the
individual crates directly:

- [`r402-core`](https://docs.rs/r402-core) — wire format, `Facilitator` trait, extension framework
- [`r402-evm`](https://docs.rs/r402-evm) — EVM chain support (EIP-3009 + Permit2 schemes)
- [`r402-svm`](https://docs.rs/r402-svm) — Solana chain support (SPL Token + Token-2022)
- [`r402-http`](https://docs.rs/r402-http) — HTTP transport (server middleware + buyer client)
- [`r402-mcp`](https://docs.rs/r402-mcp) — MCP transport (placeholder, see crate docs)

## Quick Start

```toml
[dependencies]
r402 = { version = "0.14", features = ["evm", "http", "facilitator"] }
```

## Features

| Flag             | Default | What it enables                                      |
| ---------------- | :-----: | ---------------------------------------------------- |
| `evm`            |   ✅    | Re-export `r402-evm` as [`evm`]                      |
| `svm`            |         | Re-export `r402-svm` as [`svm`]                      |
| `http`           |   ✅    | Re-export `r402-http` as [`http`]                    |
| `mcp`            |         | Re-export `r402-mcp` as [`mcp`]                      |
| `client`         |         | Forward `client` feature to all enabled chain crates |
| `server`         |         | Forward `server` feature                             |
| `facilitator`    |         | Forward `facilitator` feature                        |
| `telemetry`      |         | Enable `tracing` instrumentation                     |
| `cache`          |         | Enable in-memory caches (`moka`)                     |
| `ext-bazaar`     |         | Enable bazaar resource discovery extension           |
| `ext-payment-id` |         | Enable payment-identifier idempotency extension      |
| `all-extensions` |         | Enable every built-in extension                      |
| `full`           |         | Enable every feature (kitchen-sink)                  |

## License

Licensed under either of MIT or Apache-2.0, at your option.
