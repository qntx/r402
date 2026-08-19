# Crates

| Crate | | Description |
| --- | --- | --- |
| **[`r402`](r402/)** | [![crates.io][r402-crate]][r402-crate-url] [![docs.rs][r402-doc]][r402-doc-url] | Umbrella crate — feature-gated re-exports of the crates below |
| **[`r402-core`](r402-core/)** | [![crates.io][r402-core-crate]][r402-core-crate-url] [![docs.rs][r402-core-doc]][r402-core-doc-url] | Protocol types, scheme traits, facilitator abstractions, hooks, extensions |
| **[`r402-evm`](r402-evm/)** | [![crates.io][r402-evm-crate]][r402-evm-crate-url] [![docs.rs][r402-evm-doc]][r402-evm-doc-url] | EVM (EIP-155) — `exact` / `upto` / `auth-capture` / `batch-settlement` |
| **[`r402-solana`](r402-solana/)** | [![crates.io][r402-solana-crate]][r402-solana-crate-url] [![docs.rs][r402-solana-doc]][r402-solana-doc-url] | Solana — SPL Token / Token-2022 exact transfers |
| **[`r402-tron`](r402-tron/)** | [![crates.io][r402-tron-crate]][r402-tron-crate-url] [![docs.rs][r402-tron-doc]][r402-tron-doc-url] | Tron — TIP-712 / EIP-3009 + SUN.io Permit2 via TronGrid |
| **[`r402-casper`](r402-casper/)** | [![crates.io][r402-casper-crate]][r402-casper-crate-url] [![docs.rs][r402-casper-doc]][r402-casper-doc-url] | Casper — CEP-18 exact scheme (local preflight + remote facilitator) |
| **[`r402-near`](r402-near/)** | [![crates.io][r402-near-crate]][r402-near-crate-url] [![docs.rs][r402-near-doc]][r402-near-doc-url] | NEAR — NEP-141 / NEP-366 exact transfers |
| **[`r402-xrpl`](r402-xrpl/)** | [![crates.io][r402-xrpl-crate]][r402-xrpl-crate-url] [![docs.rs][r402-xrpl-doc]][r402-xrpl-doc-url] | XRPL — XRP / RLUSD exact transfers |
| **[`r402-hedera`](r402-hedera/)** | [![crates.io][r402-hedera-crate]][r402-hedera-crate-url] [![docs.rs][r402-hedera-doc]][r402-hedera-doc-url] | Hedera — HBAR / HTS exact transfers |
| **[`r402-algorand`](r402-algorand/)** | [![crates.io][r402-algorand-crate]][r402-algorand-crate-url] [![docs.rs][r402-algorand-doc]][r402-algorand-doc-url] | Algorand — ASA exact transfers via algod REST |
| **[`r402-keeta`](r402-keeta/)** | [![crates.io][r402-keeta-crate]][r402-keeta-crate-url] [![docs.rs][r402-keeta-doc]][r402-keeta-doc-url] | Keeta — exact `SEND` (fee-payer sponsored) |
| **[`r402-tvm`](r402-tvm/)** | [![crates.io][r402-tvm-crate]][r402-tvm-crate-url] [![docs.rs][r402-tvm-doc]][r402-tvm-doc-url] | TON — TEP-74 / W5R1 exact transfers (Highload V3 facilitator) |
| **[`r402-stellar`](r402-stellar/)** | [![crates.io][r402-stellar-crate]][r402-stellar-crate-url] [![docs.rs][r402-stellar-doc]][r402-stellar-doc-url] | Stellar — SEP-41 exact transfers (auth-entry signing) |
| **[`r402-http`](r402-http/)** | [![crates.io][r402-http-crate]][r402-http-crate-url] [![docs.rs][r402-http-doc]][r402-http-doc-url] | HTTP transport — Axum payment gate, reqwest client, facilitator client |
| **[`r402-mcp`](r402-mcp/)** | [![crates.io][r402-mcp-crate]][r402-mcp-crate-url] [![docs.rs][r402-mcp-doc]][r402-mcp-doc-url] | MCP transport on official **`rmcp`** (Go/TS parity) |

See also **[`facilitator`](https://github.com/qntx/facilitator)** — a production-ready facilitator server built on r402.

## Chain support matrix

| Chain crate | CAIP-2 examples | Schemes | Settlement model |
| --- | --- | --- | --- |
| [`r402-evm`](r402-evm/) | `eip155:8453`, `eip155:84532` | `exact`, `upto`, `auth-capture`, `batch-settlement` | In-process on-chain facilitator |
| [`r402-solana`](r402-solana/) | `solana:…` | `exact` | In-process on-chain facilitator |
| [`r402-tron`](r402-tron/) | `tron:0x2b6653dc` (mainnet), `tron:0xcd8690dc` (Nile) | `exact` | In-process via TronGrid HTTP |
| [`r402-casper`](r402-casper/) | `casper:casper`, `casper:casper-test` | `exact` | Local preflight + **remote** facilitator |
| [`r402-near`](r402-near/) | `near:mainnet`, `near:testnet` | `exact` | In-process via JSON-RPC (relayer-sponsored) |
| [`r402-xrpl`](r402-xrpl/) | `xrpl:0`, `xrpl:1`, `xrpl:2` | `exact` | In-process via JSON-RPC (payer-signed blob) |
| [`r402-hedera`](r402-hedera/) | `hedera:mainnet`, `hedera:testnet` | `exact` | In-process via Mirror REST + Hiero SDK (fee-payer-sponsored) |
| [`r402-algorand`](r402-algorand/) | `algorand:wGHE2Pwdvd7S12BL5FaOP20EGYesN73k`, `algorand:SGO1GKSzyE7IEPItTxCByw9x8FmnrCDe` | `exact` | In-process via algod REST (fee-payer-sponsored) |
| [`r402-keeta`](r402-keeta/) | `keeta:21378`, `keeta:1413829460` | `exact` | In-process via `keetanetwork-client` (fee-payer sponsored) |
| [`r402-tvm`](r402-tvm/) | `tvm:-239`, `tvm:-3` | `exact` | In-process via REST (Highload V3 relay) |
| [`r402-stellar`](r402-stellar/) | `stellar:pubnet`, `stellar:testnet` | `exact` | In-process via stellar-rpc-client 27 (fee-sponsored) |

## Dependency graph

```text
r402 (umbrella)
  ├── r402-core          (types, schemes, hooks, ResourceServer, PaymentClient, wire V2)
  ├── r402-evm           ── r402-core   exact + upto + auth-capture + batch-settlement
  ├── r402-solana        ── r402-core   exact (on-chain)
  ├── r402-tron          ── r402-core   exact (TronGrid)
  ├── r402-casper        ── r402-core   exact (preflight + remote facilitator)
  ├── r402-near          ── r402-core   exact (NEP-366 relayer)
  ├── r402-xrpl          ── r402-core   exact (payer-signed Payment)
  ├── r402-hedera        ── r402-core   exact (HBAR / HTS fee payer)
  ├── r402-algorand      ── r402-core   exact (algod REST + fee payer)
  ├── r402-keeta         ── r402-core   exact (Keeta SEND + fee payer)
  ├── r402-tvm           ── r402-core   exact (W5R1 / Highload V3)
  ├── r402-stellar       ── r402-core   exact (SEP-41 auth entries)
  ├── r402-http          ── r402-core   Axum Paygate + reqwest X402Client
  └── r402-mcp           ── r402-core   MCP transport (rmcp, V2)
```

Publish order (crates.io): `r402-core` → chain crates → `r402-http` / `r402-mcp` → `r402`.

## Feature flags

### Umbrella (`r402`)

| Feature | Default | Description |
| --- | --- | --- |
| `evm` | ✅ | Enable `r402-evm` |
| `http` | ✅ | Enable `r402-http` |
| `solana` | | Enable `r402-solana` |
| `tron` | | Enable `r402-tron` |
| `casper` | | Enable `r402-casper` |
| `near` | | Enable `r402-near` |
| `xrpl` | | Enable `r402-xrpl` |
| `hedera` | | Enable `r402-hedera` |
| `algorand` | | Enable `r402-algorand` |
| `keeta` | | Enable `r402-keeta` |
| `tvm` | | Enable `r402-tvm` |
| `stellar` | | Enable `r402-stellar` |
| `mcp` | | Enable `r402-mcp` |
| `client` | | Propagate `client` to enabled crates |
| `server` | | Propagate `server` to enabled crates |
| `facilitator` | | Propagate `facilitator` (+ Casper `http-client`) |
| `telemetry` | | `tracing` spans across enabled crates |
| `cache` | | Settlement cache in `r402-core` |
| `ext-bazaar` / `ext-payment-id` / `ext-eip2612` / `ext-erc20-approval` / `all-extensions` | | Protocol extensions |
| `full` | | All chains, transports, roles, telemetry, cache, extensions |

```toml
[dependencies]
r402 = { version = "0.16", features = [
  "evm", "solana", "tron", "casper",
  "http", "client", "server", "facilitator",
] }
```

### Per-crate roles

| Crate | `server` | `client` | `facilitator` | `telemetry` |
| --- | --- | --- | --- | --- |
| `r402-http` | Axum payment gate + facilitator client | Reqwest middleware | — | `tracing` spans |
| `r402-evm` | Price tag generation | EIP-712 / EIP-3009 / Permit2 signing | On-chain verify & settle | `tracing` spans |
| `r402-solana` | Price tag generation | SPL token signing | On-chain verify & settle | `tracing` spans |
| `r402-tron` | Price tag generation | TIP-712 / EIP-3009 / Permit2 signing | On-chain verify & settle (TronGrid) | `tracing` spans |
| `r402-casper` | Price tag generation | `SchemeClient` + EIP-712 digest signing | Remote facilitator client | `tracing` spans |
| `r402-near` | Price tag generation | NEP-366 delegate signing | On-chain verify & settle (JSON-RPC) | `tracing` spans |
| `r402-xrpl` | Price tag generation | Payer-signed `Payment` blob | On-chain verify & settle (JSON-RPC) | `tracing` spans |
| `r402-hedera` | Price tag generation | Payer-signed `TransferTransaction` | On-chain verify & settle (Mirror + Hiero) | `tracing` spans |
| `r402-algorand` | Price tag generation | Atomic group + ASA transfer signing | On-chain verify & settle (algod REST) | `tracing` spans |
| `r402-keeta` | Price tag generation | Signed `SEND` block | On-chain verify & settle (`UserClient::transmit`) | `tracing` spans |
| `r402-tvm` | Price tag generation | W5R1 settlement BoC signing | On-chain verify & Highload V3 settle | `tracing` spans |
| `r402-stellar` | Price tag generation | Auth-entry signing | On-chain verify & settle (Soroban RPC) | `tracing` spans |

[r402-crate]: https://img.shields.io/crates/v/r402.svg
[r402-crate-url]: https://crates.io/crates/r402
[r402-core-crate]: https://img.shields.io/crates/v/r402-core.svg
[r402-core-crate-url]: https://crates.io/crates/r402-core
[r402-evm-crate]: https://img.shields.io/crates/v/r402-evm.svg
[r402-evm-crate-url]: https://crates.io/crates/r402-evm
[r402-solana-crate]: https://img.shields.io/crates/v/r402-solana.svg
[r402-solana-crate-url]: https://crates.io/crates/r402-solana
[r402-tron-crate]: https://img.shields.io/crates/v/r402-tron.svg
[r402-tron-crate-url]: https://crates.io/crates/r402-tron
[r402-casper-crate]: https://img.shields.io/crates/v/r402-casper.svg
[r402-casper-crate-url]: https://crates.io/crates/r402-casper
[r402-near-crate]: https://img.shields.io/crates/v/r402-near.svg
[r402-near-crate-url]: https://crates.io/crates/r402-near
[r402-xrpl-crate]: https://img.shields.io/crates/v/r402-xrpl.svg
[r402-xrpl-crate-url]: https://crates.io/crates/r402-xrpl
[r402-hedera-crate]: https://img.shields.io/crates/v/r402-hedera.svg
[r402-hedera-crate-url]: https://crates.io/crates/r402-hedera
[r402-algorand-crate]: https://img.shields.io/crates/v/r402-algorand.svg
[r402-algorand-crate-url]: https://crates.io/crates/r402-algorand
[r402-keeta-crate]: https://img.shields.io/crates/v/r402-keeta.svg
[r402-keeta-crate-url]: https://crates.io/crates/r402-keeta
[r402-tvm-crate]: https://img.shields.io/crates/v/r402-tvm.svg
[r402-tvm-crate-url]: https://crates.io/crates/r402-tvm
[r402-stellar-crate]: https://img.shields.io/crates/v/r402-stellar.svg
[r402-stellar-crate-url]: https://crates.io/crates/r402-stellar
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
[r402-solana-doc]: https://img.shields.io/docsrs/r402-solana.svg
[r402-solana-doc-url]: https://docs.rs/r402-solana
[r402-tron-doc]: https://img.shields.io/docsrs/r402-tron.svg
[r402-tron-doc-url]: https://docs.rs/r402-tron
[r402-casper-doc]: https://img.shields.io/docsrs/r402-casper.svg
[r402-casper-doc-url]: https://docs.rs/r402-casper
[r402-near-doc]: https://img.shields.io/docsrs/r402-near.svg
[r402-near-doc-url]: https://docs.rs/r402-near
[r402-xrpl-doc]: https://img.shields.io/docsrs/r402-xrpl.svg
[r402-xrpl-doc-url]: https://docs.rs/r402-xrpl
[r402-hedera-doc]: https://img.shields.io/docsrs/r402-hedera.svg
[r402-hedera-doc-url]: https://docs.rs/r402-hedera
[r402-algorand-doc]: https://img.shields.io/docsrs/r402-algorand.svg
[r402-algorand-doc-url]: https://docs.rs/r402-algorand
[r402-keeta-doc]: https://img.shields.io/docsrs/r402-keeta.svg
[r402-keeta-doc-url]: https://docs.rs/r402-keeta
[r402-tvm-doc]: https://img.shields.io/docsrs/r402-tvm.svg
[r402-tvm-doc-url]: https://docs.rs/r402-tvm
[r402-stellar-doc]: https://img.shields.io/docsrs/r402-stellar.svg
[r402-stellar-doc-url]: https://docs.rs/r402-stellar
[r402-http-doc]: https://img.shields.io/docsrs/r402-http.svg
[r402-http-doc-url]: https://docs.rs/r402-http
[r402-mcp-doc]: https://img.shields.io/docsrs/r402-mcp.svg
[r402-mcp-doc-url]: https://docs.rs/r402-mcp
