# r402

[![CI][ci-badge]][ci-url]
[![License][license-badge]][license-url]
[![Rust][rust-badge]][rust-url]

[ci-badge]: https://github.com/qntx/r402/actions/workflows/rust.yml/badge.svg
[ci-url]: https://github.com/qntx/r402/actions/workflows/rust.yml
[license-badge]: https://img.shields.io/badge/license-MIT%2FApache--2.0-blue.svg
[license-url]: LICENSE-MIT
[rust-badge]: https://img.shields.io/badge/rust-edition%202024-orange.svg
[rust-url]: https://doc.rust-lang.org/edition-guide/

**Modular Rust SDK for the [x402 payment protocol](https://www.x402.org/) — client signing, server gating, and facilitator settlement over HTTP 402.**

r402 provides a production-grade, multi-chain implementation of the x402 protocol with dual-path ERC-3009 / Permit2 transfers, composable lifecycle hooks, MCP (Model Context Protocol) integration for AI agent payments, and 44 built-in chain deployments across EVM and Solana.

See [Security](SECURITY.md) before using in production.

## Crates

| Crate | | Description |
| --- | --- | --- |
| **[`r402`](r402/)** | [![crates.io][r402-crate]][r402-crate-url] [![docs.rs][r402-doc]][r402-doc-url] | Core library — protocol types, scheme traits, facilitator abstractions, and hook system |
| **[`r402-evm`](r402-evm/)** | [![crates.io][r402-evm-crate]][r402-evm-crate-url] [![docs.rs][r402-evm-doc]][r402-evm-doc-url] | EVM (EIP-155) — ERC-3009 transfer authorization, multi-signer management, nonce tracking |
| **[`r402-svm`](r402-svm/)** | [![crates.io][r402-svm-crate]][r402-svm-crate-url] [![docs.rs][r402-svm-doc]][r402-svm-doc-url] | Solana (SVM) — SPL token transfers, program-derived addressing |
| **[`r402-http`](r402-http/)** | [![crates.io][r402-http-crate]][r402-http-crate-url] [![docs.rs][r402-http-doc]][r402-http-doc-url] | HTTP transport — Axum payment gate middleware, reqwest client middleware, facilitator client |
| **[`r402-mcp`](r402-mcp/)** | [![crates.io][r402-mcp-crate]][r402-mcp-crate-url] [![docs.rs][r402-mcp-doc]][r402-mcp-doc-url] | MCP integration — paid tool calls for AI agents, with optional [rmcp](https://docs.rs/rmcp) support |

[r402-crate]: https://img.shields.io/crates/v/r402.svg
[r402-crate-url]: https://crates.io/crates/r402
[r402-evm-crate]: https://img.shields.io/crates/v/r402-evm.svg
[r402-evm-crate-url]: https://crates.io/crates/r402-evm
[r402-svm-crate]: https://img.shields.io/crates/v/r402-svm.svg
[r402-svm-crate-url]: https://crates.io/crates/r402-svm
[r402-http-crate]: https://img.shields.io/crates/v/r402-http.svg
[r402-http-crate-url]: https://crates.io/crates/r402-http
[r402-doc]: https://img.shields.io/docsrs/r402.svg
[r402-doc-url]: https://docs.rs/r402
[r402-evm-doc]: https://img.shields.io/docsrs/r402-evm.svg
[r402-evm-doc-url]: https://docs.rs/r402-evm
[r402-svm-doc]: https://img.shields.io/docsrs/r402-svm.svg
[r402-svm-doc-url]: https://docs.rs/r402-svm
[r402-http-doc]: https://img.shields.io/docsrs/r402-http.svg
[r402-http-doc-url]: https://docs.rs/r402-http
[r402-mcp-crate]: https://img.shields.io/crates/v/r402-mcp.svg
[r402-mcp-crate-url]: https://crates.io/crates/r402-mcp
[r402-mcp-doc]: https://img.shields.io/docsrs/r402-mcp.svg
[r402-mcp-doc-url]: https://docs.rs/r402-mcp

See also **[`facilitator`](https://github.com/qntx/facilitator)** — a production-ready facilitator server built on r402.

## Quick Start

### Protect a Route (Server)

```rust
use alloy_primitives::address;
use axum::{Router, routing::get};
use r402_evm::{Eip155Exact, USDC};
use r402_http::server::X402Middleware;

let x402 = X402Middleware::new("https://facilitator.example.com");

let app = Router::new().route(
    "/paid-content",
    get(handler).layer(
        x402.with_price_tag(Eip155Exact::price_tag(
            address!("0xYourPayToAddress"),
            USDC::base().amount(1_000_000u64), // 1 USDC (6 decimals)
        ))
    ),
);
```

### Send Payments (Client)

```rust
use alloy_signer_local::PrivateKeySigner;
use r402_evm::Eip155ExactClient;
use r402_http::client::{ReqwestWithPayments, ReqwestWithPaymentsBuild, X402Client};
use std::sync::Arc;

let signer = Arc::new("0x...".parse::<PrivateKeySigner>()?);
let x402 = X402Client::new().register(Eip155ExactClient::new(signer));

let client = reqwest::Client::new()
    .with_payments(x402)
    .build();

let res = client.get("https://api.example.com/paid").send().await?;
```

## Composable Payment API

The standard `X402Middleware` Tower layer handles the entire payment lifecycle (verify → execute → settle) as an opaque unit. This works well for simple request/response APIs, but **blocks streaming responses** (SSE, chunked transfer) because the `Payment-Response` header must be computed *after* on-chain settlement completes — forcing the server to buffer the entire response body until the blockchain confirms the transaction (typically 2–5 s).

To solve this, `r402-http` exposes a **composable API** that lets the application control *when* settlement happens:

```rust
use r402_http::server::{Paygate, VerifiedPayment, settlement_to_header};

// Step 1 — Build the gate once (reusable across requests).
let gate = Paygate::builder(facilitator)
    .accepts(price_tags)
    .resource(resource_info)
    .build();

// Step 2 — Verify the payment (off-chain, ~200 ms).
let verified: VerifiedPayment = gate.verify_only(req.headers()).await?;

// Step 3 — Execute the inner handler / stream the response to the client.
let response = forward_request(&req).await?;

// Step 4 — Settle at your discretion (on-chain, 2–5 s).
let settlement = verified.settle(&facilitator).await?;
let header = settlement_to_header(&settlement)?;
```

### Key design decisions

- **`VerifiedPayment` is `Send + 'static`** — no lifetime parameters, so it can be stored in Axum request extensions, moved across `await` points, or sent to a background task.
- **`settle(self)` consumes ownership** — prevents double-settlement at the type level without runtime checks.
- **`SettleResponse::encode_base64()` validates success** — returns `None` for error variants, preventing accidental encoding of failed settlements into `Payment-Response` headers.
- **`handle_request` / `handle_request_fallible` are unchanged** — they now delegate to `verify_only` + `VerifiedPayment::settle` internally (DRY), so existing Layer-based integrations continue to work without modification.

### Settlement strategies

| Strategy | When to use | Header delivery |
| --- | --- | --- |
| **Synchronous** (default) | Non-streaming APIs | `Payment-Response` HTTP header |
| **Deferred** | SSE / chunked streaming | Appended as final SSE event after stream ends |

The `X402Middleware` Tower layer always uses the synchronous strategy. For deferred settlement, use `verify_only` + `settle` directly in your handler.

## Design

| Aspect | Details |
| --- | --- |
| Built-in chains | **44** — 42 EVM (EIP-155) + 2 Solana |
| Transfer methods | **Dual path** — ERC-3009 `transferWithAuthorization` + Permit2 proxy |
| Lifecycle hooks | **`FacilitatorHooks`** (verify/settle) + **`ClientHooks`** (payment creation) |
| Async model | **Zero `async_trait`** in core — RPITIT / `Pin<Box<dyn Future>>` |
| Facilitator trait | **Unified** — dyn-compatible `Box<dyn Facilitator>` across all schemes |
| Wire format | **V2-only** server (CAIP-2 chain IDs, `Payment-Signature` header) |
| Settlement errors | **Explicit** — failed settle returns 402 with structured error |
| Network definitions | **Decoupled** — per-chain crate (`r402-evm`, `r402-svm`) |
| Smart wallets | **EIP-6492** (counterfactual) + **EIP-1271** (deployed) + **ERC-2098** (compact signatures) |
| Linting | **`pedantic` + `nursery` + `correctness`** (deny) |

## Feature Flags

Each chain and transport crate uses feature flags to minimize compile-time dependencies:

| Crate | `server` | `client` | `facilitator` | `telemetry` |
| --- | --- | --- | --- | --- |
| `r402-http` | Axum payment gate + facilitator client | Reqwest middleware | — | `tracing` spans |
| `r402-evm` | Price tag generation | EIP-712 / EIP-3009 / Permit2 signing | On-chain verify & settle | `tracing` spans |
| `r402-svm` | Price tag generation | SPL token signing | On-chain verify & settle | `tracing` spans |
| `r402-mcp` | — | — | — | `tracing` spans |

`r402-mcp` uses `rmcp` (optional) for built-in [rmcp](https://docs.rs/rmcp) SDK integration.

## Security

See [`SECURITY.md`](SECURITY.md) for disclaimers, supported versions, and vulnerability reporting.

## Acknowledgments

- [x402 Protocol Specification](https://www.x402.org/) — protocol design by Coinbase
- [coinbase/x402](https://github.com/coinbase/x402) — official reference implementations (TypeScript, Python, Go)
- [x402-rs/x402-rs](https://github.com/x402-rs/x402-rs) — community Rust implementation

## License

Licensed under either of:

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or <https://www.apache.org/licenses/LICENSE-2.0>)
- MIT License ([LICENSE-MIT](LICENSE-MIT) or <https://opensource.org/licenses/MIT>)

at your option.

Unless you explicitly state otherwise, any contribution intentionally submitted for inclusion in this project shall be dual-licensed as above, without any additional terms or conditions.
