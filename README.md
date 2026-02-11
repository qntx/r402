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

This repository is based on [x402-rs] and includes architectural improvements for internal use at qntx. For the upstream community implementation, see the [original project][x402-rs].

[Architecture](ARCHITECTURE.md)
| [x402 Protocol Spec](https://www.x402.org/)
| [Upstream (x402-rs)][x402-rs]

[x402-rs]: https://github.com/x402-rs/x402-rs

> [!WARNING]
> This software has **not** been audited. See [Security](#security) before using in production.

## Crates

| Crate | Description |
| --- | --- |
| **[`r402`](r402/)** | Core library — protocol types, scheme traits, facilitator abstractions, and hook system |
| **[`r402-evm`](r402-evm/)** | EVM (EIP-155) — ERC-3009 transfer authorization, multi-signer management, nonce tracking |
| **[`r402-svm`](r402-svm/)** | Solana (SVM) — SPL token transfers, program-derived addressing |
| **[`r402-http`](r402-http/)** | HTTP transport — Axum payment gate middleware, reqwest client middleware, facilitator client |

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

r402 diverges from [x402-rs] in several architectural areas:

- **Unified `Facilitator` trait** — dyn-compatible with a single `FacilitatorError` enum, enabling `Box<dyn Facilitator>` and heterogeneous composition (local handlers + remote clients + hook decorators in one registry)
- **Zero `async_trait`** — all core traits use native RPITIT or manual `Pin<Box<dyn Future>>`, eliminating the proc-macro dependency entirely
- **Lifecycle hooks** — `FacilitatorHooks` trait + `HookedFacilitator` decorator with before/after/failure callbacks for verify and settle, mirroring the [official Go SDK](https://github.com/coinbase/x402)
- **V2-only server** — the payment gate implements only the V2 wire format (CAIP-2 chain IDs, `Payment-Signature` header); V1 compatibility is confined to the facilitator layer
- **Settlement validation** — `SettleResponse::Error` is explicitly checked after every settlement; failed settlements return `500`, not `402`
- **Decoupled networks** — chain definitions live in `r402-evm` / `r402-svm`, not in the core crate; the core exposes only `NetworkRegistry` abstractions
- **Strict linting** — `clippy::pedantic` + `clippy::nursery` + `clippy::correctness` (deny) across all crates, zero warnings

## Feature Flags

Each chain and transport crate uses feature flags to minimize compile-time dependencies:

| Crate | `server` | `client` | `facilitator` | `telemetry` |
| --- | --- | --- | --- | --- |
| `r402-http` | Axum payment gate | Reqwest middleware | HTTP facilitator client | `tracing` spans |
| `r402-evm` | Price tag generation | EIP-712 / EIP-3009 signing | On-chain verify & settle | `tracing` spans |
| `r402-svm` | Price tag generation | SPL token signing | On-chain verify & settle | `tracing` spans |

## Security

> [!CAUTION]
> **This software has NOT been audited by any independent security firm.**

This library interacts with blockchain networks and processes real financial transactions. Bugs or vulnerabilities **may result in irreversible loss of funds**.

- **No warranty.** Provided "AS IS" without warranty of any kind, express or implied, including but not limited to merchantability, fitness for a particular purpose, and non-infringement.
- **Unaudited.** The codebase has not undergone a formal security audit. Undiscovered vulnerabilities may exist despite extensive testing and strict linting.
- **Use at your own risk.** The authors and contributors accept no responsibility for financial losses, damages, or other liabilities arising from the use of this software.
- **Testnet first.** Always validate on testnets before deploying to mainnet.
- **Key management.** Users are solely responsible for the secure handling of private keys and signing credentials.

To report a vulnerability, please open a [GitHub Security Advisory](https://github.com/qntx/r402/security/advisories/new) — do not file a public issue.

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
