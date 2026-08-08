# Crates

| Crate | | Description |
| --- | --- | --- |
| **[`r402`](r402/)** | [![crates.io][r402-crate]][r402-crate-url] [![docs.rs][r402-doc]][r402-doc-url] | Umbrella crate — feature-gated re-exports of the crates below |
| **[`r402-core`](r402-core/)** | [![crates.io][r402-core-crate]][r402-core-crate-url] [![docs.rs][r402-core-doc]][r402-core-doc-url] | Protocol types, scheme traits, facilitator abstractions, hooks, extensions |
| **[`r402-evm`](r402-evm/)** | [![crates.io][r402-evm-crate]][r402-evm-crate-url] [![docs.rs][r402-evm-doc]][r402-evm-doc-url] | EVM (EIP-155) — ERC-3009 + Permit2, `exact` + `upto`, multi-signer / nonce |
| **[`r402-svm`](r402-svm/)** | [![crates.io][r402-svm-crate]][r402-svm-crate-url] [![docs.rs][r402-svm-doc]][r402-svm-doc-url] | Solana (SVM) — SPL Token / Token-2022 exact transfers |
| **[`r402-tron`](r402-tron/)** | [![crates.io][r402-tron-crate]][r402-tron-crate-url] [![docs.rs][r402-tron-doc]][r402-tron-doc-url] | Tron — TIP-712 / EIP-3009 + SUN.io Permit2 via TronGrid |
| **[`r402-casper`](r402-casper/)** | [![crates.io][r402-casper-crate]][r402-casper-crate-url] [![docs.rs][r402-casper-doc]][r402-casper-doc-url] | Casper — CEP-18 exact scheme (local preflight + remote facilitator) |
| **[`r402-http`](r402-http/)** | [![crates.io][r402-http-crate]][r402-http-crate-url] [![docs.rs][r402-http-doc]][r402-http-doc-url] | HTTP transport — Axum payment gate, reqwest client, facilitator client |
| **[`r402-mcp`](r402-mcp/)** | [![crates.io][r402-mcp-crate]][r402-mcp-crate-url] [![docs.rs][r402-mcp-doc]][r402-mcp-doc-url] | MCP transport — meta keys + payment wrapper / auto-pay (SDK-agnostic) |

See also **[`facilitator`](https://github.com/qntx/facilitator)** — a production-ready facilitator server built on r402.

## Chain support matrix

| Chain crate | CAIP-2 examples | Schemes | Settlement model |
| --- | --- | --- | --- |
| [`r402-evm`](r402-evm/) | `eip155:8453`, `eip155:84532` | `exact`, `upto` | In-process on-chain facilitator |
| [`r402-svm`](r402-svm/) | `solana:…` | `exact` | In-process on-chain facilitator |
| [`r402-tron`](r402-tron/) | `tron:0x2b6653dc` (mainnet), Nile, Shasta | `exact` | In-process via TronGrid HTTP |
| [`r402-casper`](r402-casper/) | `casper:casper`, `casper:casper-test` | `exact` | Local preflight + **remote** facilitator |

## Dependency graph

```text
r402 (umbrella)
  ├── r402-core          (types, schemes, hooks, wire V2)
  ├── r402-evm           ── r402-core   exact + upto (on-chain)
  ├── r402-svm           ── r402-core   exact (on-chain)
  ├── r402-tron          ── r402-core   exact (TronGrid)
  ├── r402-casper        ── r402-core   exact (preflight + remote facilitator)
  ├── r402-http          ── r402-core   Axum gate + reqwest client
  └── r402-mcp           ── r402-core   MCP transport (placeholder)
```

Publish order (crates.io): `r402-core` → chain crates → `r402-http` / `r402-mcp` → `r402`.

## Feature flags

### Umbrella (`r402`)

| Feature | Default | Description |
| --- | --- | --- |
| `evm` | ✅ | Enable `r402-evm` |
| `http` | ✅ | Enable `r402-http` |
| `svm` | | Enable `r402-svm` |
| `tron` | | Enable `r402-tron` |
| `casper` | | Enable `r402-casper` |
| `mcp` | | Enable `r402-mcp` |
| `client` | | Propagate `client` to enabled crates |
| `server` | | Propagate `server` to enabled crates |
| `facilitator` | | Propagate `facilitator` (+ Casper `http-client`) |
| `telemetry` | | `tracing` spans across enabled crates |
| `cache` | | Settlement cache in `r402-core` |
| `ext-bazaar` / `ext-payment-id` / `all-extensions` | | Protocol extensions |
| `full` | | All chains, transports, roles, telemetry, cache, extensions |

```toml
[dependencies]
r402 = { version = "0.14", features = [
  "evm", "svm", "tron", "casper",
  "http", "client", "server", "facilitator",
] }
```

### Per-crate roles

| Crate | `server` | `client` | `facilitator` | `telemetry` |
| --- | --- | --- | --- | --- |
| `r402-http` | Axum payment gate + facilitator client | Reqwest middleware | — | `tracing` spans |
| `r402-evm` | Price tag generation | EIP-712 / EIP-3009 / Permit2 signing | On-chain verify & settle | `tracing` spans |
| `r402-svm` | Price tag generation | SPL token signing | On-chain verify & settle | `tracing` spans |
| `r402-tron` | Price tag generation | TIP-712 / EIP-3009 / Permit2 signing | On-chain verify & settle (TronGrid) | `tracing` spans |
| `r402-casper` | Price tag generation | `SchemeClient` + EIP-712 digest signing | Remote facilitator client | `tracing` spans |

[r402-crate]: https://img.shields.io/crates/v/r402.svg
[r402-crate-url]: https://crates.io/crates/r402
[r402-core-crate]: https://img.shields.io/crates/v/r402-core.svg
[r402-core-crate-url]: https://crates.io/crates/r402-core
[r402-evm-crate]: https://img.shields.io/crates/v/r402-evm.svg
[r402-evm-crate-url]: https://crates.io/crates/r402-evm
[r402-svm-crate]: https://img.shields.io/crates/v/r402-svm.svg
[r402-svm-crate-url]: https://crates.io/crates/r402-svm
[r402-tron-crate]: https://img.shields.io/crates/v/r402-tron.svg
[r402-tron-crate-url]: https://crates.io/crates/r402-tron
[r402-casper-crate]: https://img.shields.io/crates/v/r402-casper.svg
[r402-casper-crate-url]: https://crates.io/crates/r402-casper
[r402-http-crate]: https://img.shields.io/crates/v/r402-http.svg
[r402-http-crate-url]: https://crates.io/crates/r402-http
[r402-mcp-crate]: https://img.shields.io/crates/v/r402-mcp.svg
[r402-mcp-crate-url]: https://crates.io/crates/r402-mcp
[r402-doc]: https://img.shields.io/docsrs/r402.svg
[r402-doc-url]: https://docs.rs/r402
[r402-core-doc]: https://img.shields.io/docsrs/r402-core.svg
[r402-core-doc-url]: https://docs.rs/r402-core
[r402-evm-doc]: https://img.shields.io/docsrs/r402-evm.svg
[r402-evm-doc-url]: https://docs.rs/r402-evm
[r402-svm-doc]: https://img.shields.io/docsrs/r402-svm.svg
[r402-svm-doc-url]: https://docs.rs/r402-svm
[r402-tron-doc]: https://img.shields.io/docsrs/r402-tron.svg
[r402-tron-doc-url]: https://docs.rs/r402-tron
[r402-casper-doc]: https://img.shields.io/docsrs/r402-casper.svg
[r402-casper-doc-url]: https://docs.rs/r402-casper
[r402-http-doc]: https://img.shields.io/docsrs/r402-http.svg
[r402-http-doc-url]: https://docs.rs/r402-http
[r402-mcp-doc]: https://img.shields.io/docsrs/r402-mcp.svg
[r402-mcp-doc-url]: https://docs.rs/r402-mcp
