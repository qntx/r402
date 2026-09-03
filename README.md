<!-- markdownlint-disable MD033 MD041 MD036 -->

# R402

[![Crates.io][crates-badge]][crates-url]
[![Docs.rs][docs-badge]][docs-url]
[![CI][ci-badge]][ci-url]
[![License][license-badge]][license-url]
[![Rust][rust-badge]][rust-url]

[crates-badge]: https://img.shields.io/crates/v/r402.svg
[crates-url]: https://crates.io/crates/r402
[docs-badge]: https://img.shields.io/docsrs/r402.svg
[docs-url]: https://docs.rs/r402
[ci-badge]: https://github.com/qntx/r402/actions/workflows/ci.yml/badge.svg
[ci-url]: https://github.com/qntx/r402/actions/workflows/ci.yml
[license-badge]: https://img.shields.io/badge/license-MIT%2FApache--2.0-blue.svg
[license-url]: LICENSE-MIT
[rust-badge]: https://img.shields.io/badge/rust-edition%202024-orange.svg
[rust-url]: https://doc.rust-lang.org/edition-guide/

**Modular Rust SDK for the [x402 payment protocol](https://www.x402.org/) — client signing, server gating, and facilitator settlement over HTTP 402.**

Protocol version **2** only. Schemes `exact` / `upto`. Deployments: **EVM, Solana, Tron, Casper, NEAR, XRPL, Hedera, Algorand, Aptos, Keeta, TON, Stellar**.

> **See also** [`facilitator`](https://github.com/qntx/facilitator) — a production-ready facilitator server built on r402.

## Quick Start

### Install

```toml
[dependencies]
r402 = { version = "0.18", features = ["evm", "http"] }
```

`default = ["evm", "http"]` opens each crate's `client` and `server` features.
Do not pass `client` / `server` on the umbrella.

Full feature matrix and crate list: **[`crates/README.md`](crates/README.md)**.

### Protect a route

`Eip155Exact::price_tag` is three arguments. `with_price_tag` returns
`Result<X402Layer, BuildError>` (`BuildError::MissingBaseUrl` when `base_url`
was omitted). There is no `X402Route` and no `into_layer`.

Consumer binaries typically return `anyhow::Result`.

```rust,no_run
fn paid_router() -> Result<axum::Router, Box<dyn std::error::Error>> {
    use alloy_primitives::address;
    use axum::{Router, routing::get};
    use r402::evm::{Eip155Exact, USDC};
    use r402::http::server::{SettlementMode, X402Middleware};

    async fn handler() -> &'static str {
        "ok"
    }

    let x402 = X402Middleware::try_new("https://facilitator.example.com")?
        .with_base_url("https://api.example.com".parse()?)
        .with_scheme("eip155:*".parse()?, Eip155Exact);

    let app = Router::new().route(
        "/paid-content",
        get(handler).layer(
            x402.with_price_tag(Eip155Exact::price_tag(
                address!("0x1111111111111111111111111111111111111111"),
                USDC::base().amount(1_000_000u64),
                None,
            ))?
            .with_settlement_mode(SettlementMode::Sequential),
        ),
    );
    Ok(app)
}
```

```text
X402Middleware::try_new(url)?
    .with_base_url(origin)
    .with_scheme(pattern, Eip155Exact)
    .with_price_tag(tag)?
        .with_settlement_mode(mode)
        .with_resource(url)
        .with_description(s)
        .with_mime_type(s)
        .with_hooks(h)
```

### Send a payment (TARGET)

HTTP buyer retry (`WithPayments`, `X402Client`) is the intended client
surface. Consumer binaries typically return `anyhow::Result`.

```rust,ignore
async fn pay() -> anyhow::Result<reqwest::Response> {
    use alloy_signer_local::PrivateKeySigner;
    use r402::evm::Eip155ExactClient;
    use r402::http::{WithPayments, X402Client};
    use std::sync::Arc;

    let signer = Arc::new("0x...".parse::<PrivateKeySigner>()?);
    let client = reqwest::Client::new().with_payments(
        X402Client::new().register(Eip155ExactClient::new(signer)),
    );
    Ok(client.get("https://api.example.com/paid").send().await?)
}
```

## Settlement modes

Configurable on `X402Layer` via `with_settlement_mode`. Orthogonal to
`extra.paymentFlow`.

| Mode | After verify | Receipt | Handler failure | `upto` actual |
| --- | --- | --- | --- | --- |
| **Sequential** (default) | execute, then settle | attached | settle not started | applied |
| **Concurrent** | settle ∥ execute, then join | attached | drop settle, do not await | 402 |
| **Background** | spawn settle, execute, return | not attached | already spawned | strip; charge signed max |

`upto` actual is Sequential only. Concurrent or Background with
upfront/escrow is 500.

## `upto` (TARGET)

EVM `upto` (Permit2 witness: verify = max, settle = actual ≤ max) and
Solana `upto` (escrow) live under each chain crate's `src/upto/`. EVM
`Eip155Upto::price_tag(pay_to, asset)` is two arguments (max amount).
Handlers opt in with `UptoActualAmount` on Sequential responses.

## Wire

| Item | Contract |
| --- | --- |
| Version | `x402Version: 2` |
| 402 challenge | `Payment-Required` (base64, Title-Case) |
| Retry | `Payment-Signature` |
| Receipt | `Payment-Response` |
| JSON | camelCase, `deny_unknown_fields` |

| Situation | Status |
| --- | --- |
| Unpaid / verify fail / settle fail / malformed header | 402 |
| Permit2 allowance | 412 |
| Incompatible mode / missing scheme | 500 |
| Facilitator transport | 502 |
| Success | 200 |

## Design

- **Chains** — EVM (EIP-155), Solana, Tron, Casper, NEAR, TON, XRPL, Hedera, Algorand, Aptos, Keeta, Stellar
- **Schemes** — `exact` (all chains) + `upto` (EVM Permit2 witness, Solana escrow)
- **Transfer methods** — ERC-3009 / Permit2 (EVM, Tron); SPL (SVM); CEP-18 (Casper); NEP-141 / NEP-366 (NEAR); TEP-74 / W5R1 (TON); XRP / RLUSD Payment (XRPL); HBAR / HTS (Hedera); ASA (Algorand); FA (Aptos); SEND (Keeta); SEP-41 (Stellar)
- **Wire** — V2-only (CAIP-2 chain IDs, `Payment-*` headers)
- **Smart wallets** — EIP-6492 (counterfactual) + EIP-1271 (deployed) + ERC-2098 (compact)

## Crates

See **[`crates/README.md`](crates/README.md)** for the crate table, chain matrix, and feature flags.

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

---

<div align="center">

A **[QuantX](https://qntx.org)** open-source project.

<a href="https://qntx.org"><img alt="QuantX" width="369" src="https://raw.githubusercontent.com/qntx/.github/main/profile/qntx.svg" /></a>

Code is law. We write both.

</div>
