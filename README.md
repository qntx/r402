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

r402 is a comprehensive restructuring of [x402-rs], fully aligned with the [official][x402-sdk] feature set — adding Permit2 transfers, lifecycle hooks, and 44 built-in chain deployments. For the upstream community implementation, see [x402-rs].

[x402-rs]: https://github.com/x402-rs/x402-rs
[x402-sdk]: https://github.com/coinbase/x402

See [Security](SECURITY.md) before using in production.

## Crates

| Crate | | Description |
| --- | --- | --- |
| **[`r402`](r402/)** | [![crates.io][r402-crate]][r402-crate-url] [![docs.rs][r402-doc]][r402-doc-url] | Core library — protocol types, scheme traits, facilitator abstractions, and hook system |
| **[`r402-evm`](r402-evm/)** | [![crates.io][r402-evm-crate]][r402-evm-crate-url] [![docs.rs][r402-evm-doc]][r402-evm-doc-url] | EVM (EIP-155) — ERC-3009 transfer authorization, multi-signer management, nonce tracking |
| **[`r402-svm`](r402-svm/)** | [![crates.io][r402-svm-crate]][r402-svm-crate-url] [![docs.rs][r402-svm-doc]][r402-svm-doc-url] | Solana (SVM) — SPL token transfers, program-derived addressing |
| **[`r402-http`](r402-http/)** | [![crates.io][r402-http-crate]][r402-http-crate-url] [![docs.rs][r402-http-doc]][r402-http-doc-url] | HTTP transport — Axum payment gate middleware, reqwest client middleware, facilitator client |

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

## Design

| | r402 | x402-rs |
| --- | --- | --- |
| Built-in chains | **44** (42 EVM + 2 Solana) | 18 (14 EVM + 2 Solana + 2 Aptos) |
| Permit2 | **Dual path** — ERC-3009 + `x402Permit2Proxy` | ERC-3009 only |
| Lifecycle hooks | **`FacilitatorHooks`** + **`ClientHooks`** | None |
| `async_trait` | **Zero** — RPITIT / `Pin<Box<dyn Future>>` | Required |
| Facilitator trait | **Unified** — dyn-compatible `Box<dyn Facilitator>` | Separate per-scheme |
| Server wire format | **V2-only** (CAIP-2, `Payment-Signature` header) | V1 + V2 |
| Settlement errors | **Explicit** — failed settle → `500` | Silent |
| Network definitions | **Decoupled** — per-chain crate | Core crate |
| Linting | **`pedantic` + `nursery` + `correctness`** (deny) | Default |

## Feature Flags

Each chain and transport crate uses feature flags to minimize compile-time dependencies:

| Crate | `server` | `client` | `facilitator` | `telemetry` |
| --- | --- | --- | --- | --- |
| `r402-http` | Axum payment gate | Reqwest middleware | HTTP facilitator client | `tracing` spans |
| `r402-evm` | Price tag generation | EIP-712 / EIP-3009 signing | On-chain verify & settle | `tracing` spans |
| `r402-svm` | Price tag generation | SPL token signing | On-chain verify & settle | `tracing` spans |

## Security

See [`SECURITY.md`](SECURITY.md) for disclaimers, supported versions, and vulnerability reporting.

## Acknowledgments

- [x402-rs/x402-rs](https://github.com/x402-rs/x402-rs) — upstream Rust implementation (community)
- [x402 Protocol Specification](https://www.x402.org/) — protocol design by Coinbase
- [coinbase/x402](https://github.com/coinbase/x402) — official reference implementations (TypeScript, Python, Go)

## License

Licensed under either of:

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or <https://www.apache.org/licenses/LICENSE-2.0>)
- MIT License ([LICENSE-MIT](LICENSE-MIT) or <https://opensource.org/licenses/MIT>)

at your option.

Unless you explicitly state otherwise, any contribution intentionally submitted for inclusion in this project shall be dual-licensed as above, without any additional terms or conditions.
