# Crates

Workspace **0.21.0**. Path dependencies match the graph below. Umbrella
`default = ["evm", "http"]`. Other production chains and MCP are umbrella
features (`svm`, `near`, `xrpl`, `hedera`, `avm`, `aptos`, `keeta`, `tvm`,
`stellar`, `concordium`, `mcp`). `full` enables those plus `client` /
`server` / `facilitator` / `telemetry`. `tron` and `casper` are opt-in,
not in `full`. **[`r402-contract`](r402-contract/)** is an unpublished
workspace member — not on crates.io, not in `publish.yml`. crates.io
**`r402-core` 0.17.1** is not yanked.

SIWX (`siwx` / `siwx-evm` / `siwx-svm`) is crates.io **0.6.1**.

| Crate | | Description |
| --- | --- | --- |
| **[`r402`](r402/)** | [![crates.io][r402-crate]][r402-crate-url] [![docs.rs][r402-doc]][r402-doc-url] | Umbrella crate — feature-gated re-exports of the crates below |
| **[`r402-protocol`](r402-protocol/)** | [![crates.io][r402-protocol-crate]][r402-protocol-crate-url] [![docs.rs][r402-protocol-doc]][r402-protocol-doc-url] | Wire types, CAIP-2, scheme markers, money, metrics names |
| **[`r402-client`](r402-client/)** | [![crates.io][r402-client-crate]][r402-client-crate-url] [![docs.rs][r402-client-doc]][r402-client-doc-url] | Buyer register / select / spendControls |
| **[`r402-server`](r402-server/)** | [![crates.io][r402-server-crate]][r402-server-crate-url] [![docs.rs][r402-server-doc]][r402-server-doc-url] | Resource server, paymentFlow, authorization SettlementMode scheduler |
| **[`r402-facilitator`](r402-facilitator/)** | [![crates.io][r402-facilitator-crate]][r402-facilitator-crate-url] [![docs.rs][r402-facilitator-doc]][r402-facilitator-doc-url] | Facilitator trait, cache, pending settlement, remote HTTP client |
| **[`r402-extensions`](r402-extensions/)** | [![crates.io][r402-extensions-crate]][r402-extensions-crate-url] [![docs.rs][r402-extensions-doc]][r402-extensions-doc-url] | Protocol extension implementations |
| **[`r402-evm`](r402-evm/)** | [![crates.io][r402-evm-crate]][r402-evm-crate-url] [![docs.rs][r402-evm-doc]][r402-evm-doc-url] | EVM (EIP-155) — `exact` / `upto` / `auth-capture` / `batch-settlement` |
| **[`r402-svm`](r402-svm/)** | [![crates.io][r402-svm-crate]][r402-svm-crate-url] [![docs.rs][r402-svm-doc]][r402-svm-doc-url] | SVM (Solana) — SPL Token / Token-2022 exact + upto escrow |
| **[`r402-tron`](r402-tron/)** | [![crates.io][r402-tron-crate]][r402-tron-crate-url] [![docs.rs][r402-tron-doc]][r402-tron-doc-url] | **Experimental / not an x402 protocol mechanism.** TIP-712 / EIP-3009 + SUN.io Permit2 via TronGrid. Umbrella feature `tron`, not in `full` |
| **[`r402-casper`](r402-casper/)** | [![crates.io][r402-casper-crate]][r402-casper-crate-url] [![docs.rs][r402-casper-doc]][r402-casper-doc-url] | Casper — CEP-18 exact (local preflight + remote facilitator) |
| **[`r402-near`](r402-near/)** | [![crates.io][r402-near-crate]][r402-near-crate-url] [![docs.rs][r402-near-doc]][r402-near-doc-url] | NEAR — NEP-141 / NEP-366 exact transfers |
| **[`r402-xrpl`](r402-xrpl/)** | [![crates.io][r402-xrpl-crate]][r402-xrpl-crate-url] [![docs.rs][r402-xrpl-doc]][r402-xrpl-doc-url] | XRPL — XRP / RLUSD exact transfers |
| **[`r402-hedera`](r402-hedera/)** | [![crates.io][r402-hedera-crate]][r402-hedera-crate-url] [![docs.rs][r402-hedera-doc]][r402-hedera-doc-url] | Hedera — HBAR / HTS exact transfers |
| **[`r402-avm`](r402-avm/)** | [![crates.io][r402-avm-crate]][r402-avm-crate-url] [![docs.rs][r402-avm-doc]][r402-avm-doc-url] | AVM (Algorand) — ASA exact transfers via algod REST |
| **[`r402-aptos`](r402-aptos/)** | [![crates.io][r402-aptos-crate]][r402-aptos-crate-url] [![docs.rs][r402-aptos-doc]][r402-aptos-doc-url] | Aptos — FA exact transfers via aptos-sdk 0.6 |
| **[`r402-keeta`](r402-keeta/)** | [![crates.io][r402-keeta-crate]][r402-keeta-crate-url] [![docs.rs][r402-keeta-doc]][r402-keeta-doc-url] | Keeta — exact `SEND` (fee-payer sponsored) |
| **[`r402-tvm`](r402-tvm/)** | [![crates.io][r402-tvm-crate]][r402-tvm-crate-url] [![docs.rs][r402-tvm-doc]][r402-tvm-doc-url] | TON — TEP-74 / W5R1 exact transfers (Highload V3 facilitator) |
| **[`r402-stellar`](r402-stellar/)** | [![crates.io][r402-stellar-crate]][r402-stellar-crate-url] [![docs.rs][r402-stellar-doc]][r402-stellar-doc-url] | Stellar — SEP-41 exact transfers (auth-entry signing) |
| **[`r402-concordium`](r402-concordium/)** | [![crates.io][r402-concordium-crate]][r402-concordium-crate-url] [![docs.rs][r402-concordium-doc]][r402-concordium-doc-url] | Concordium — CCD / PLT exact sponsored V1 |
| **[`r402-http`](r402-http/)** | [![crates.io][r402-http-crate]][r402-http-crate-url] [![docs.rs][r402-http-doc]][r402-http-doc-url] | HTTP transport — Axum layer, reqwest buyer |
| **[`r402-mcp`](r402-mcp/)** | [![crates.io][r402-mcp-crate]][r402-mcp-crate-url] [![docs.rs][r402-mcp-doc]][r402-mcp-doc-url] | MCP transport on official **`rmcp`** (Go/TS parity) |
| **[`r402-contract`](r402-contract/)** | unpublished | JSON contract assertions. Workspace member only. Not published. |

See also **[`facilitator`](https://github.com/qntx/facilitator)** — a production-ready facilitator server built on r402.

## Chain support matrix

| Chain crate | CAIP-2 examples | Schemes | Settlement model |
| --- | --- | --- | --- |
| [`r402-evm`](r402-evm/) | `eip155:8453`, `eip155:84532` | `exact`, `upto`, `auth-capture`, `batch-settlement` | In-process on-chain facilitator |
| [`r402-svm`](r402-svm/) | `solana:…` | `exact`, `upto` | In-process on-chain facilitator |
| [`r402-tron`](r402-tron/) | `tron:0x2b6653dc` (mainnet), `tron:0xcd8690dc` (Nile) | `exact` (experimental / not protocol) | In-process via TronGrid HTTP |
| [`r402-casper`](r402-casper/) | `casper:casper`, `casper:casper-test` | `exact` | Local preflight + **remote** facilitator (`R402_CASPER_FACILITATOR_URL`) |
| [`r402-near`](r402-near/) | `near:mainnet`, `near:testnet` | `exact` | In-process via JSON-RPC (relayer-sponsored) |
| [`r402-xrpl`](r402-xrpl/) | `xrpl:0`, `xrpl:1`, `xrpl:2` | `exact` | In-process via JSON-RPC (payer-signed blob) |
| [`r402-hedera`](r402-hedera/) | `hedera:mainnet`, `hedera:testnet` | `exact` | In-process via Mirror REST + Hiero SDK (fee-payer-sponsored) |
| [`r402-avm`](r402-avm/) | `algorand:wGHE2Pwdvd7S12BL5FaOP20EGYesN73k`, `algorand:SGO1GKSzyE7IEPItTxCByw9x8FmnrCDe` | `exact` | In-process via algod REST (fee-payer-sponsored) |
| [`r402-aptos`](r402-aptos/) | `aptos:1`, `aptos:2` | `exact` | In-process via `aptos-sdk` 0.6 (fee-payer-sponsored) |
| [`r402-keeta`](r402-keeta/) | `keeta:21378`, `keeta:1413829460` | `exact` | In-process via `keetanetwork-client` (fee-payer sponsored) |
| [`r402-tvm`](r402-tvm/) | `tvm:-239`, `tvm:-3` | `exact` | In-process via REST (Highload V3 relay) |
| [`r402-stellar`](r402-stellar/) | `stellar:pubnet`, `stellar:testnet` | `exact` | In-process via stellar-rpc-client 27 (fee-sponsored) |
| [`r402-concordium`](r402-concordium/) | `ccd:9dd9ca4d19e9393877d2c44b70f89acb`, `ccd:4221332d34e1694168c2a0c0b3fd0f27` | `exact` | In-process via concordium-rust-sdk 9 (sponsored V1). Umbrella feature `concordium`, in `full`, not default |

## Dependency graph

Arrow means Cargo `depends on`. Solid = always. Dashed = feature-gated (`client` / `server` / `facilitator` / `siwx` / umbrella chain features). `r402-evm` always depends on `r402-client`; other chains only do when `client` is on.

```mermaid
flowchart TB
  umbrella["r402"]
  contract["r402-contract"]

  subgraph transports["Transports"]
    http["r402-http"]
    mcp["r402-mcp"]
  end

  subgraph chains["Chain crates"]
    evm["r402-evm"]
    svm["r402-svm"]
    prod["near · xrpl · hedera · avm<br/>aptos · keeta · tvm · stellar · concordium"]
    extra["r402-tron · r402-casper"]
  end

  subgraph roles["Buyer / resource / facilitator"]
    client["r402-client"]
    server["r402-server"]
    facilitator["r402-facilitator"]
    extensions["r402-extensions"]
  end

  protocol["r402-protocol"]
  siwx["siwx 0.6"]

  client --> protocol
  facilitator --> protocol
  server --> protocol
  server --> facilitator
  extensions --> protocol
  extensions -.-> facilitator
  extensions -.-> client
  extensions -.-> siwx

  evm --> protocol
  evm --> client
  evm -.-> server
  evm -.-> facilitator
  evm -.-> extensions

  svm --> protocol
  svm -.-> client
  svm -.-> server
  svm -.-> facilitator

  prod --> protocol
  prod -.-> client
  prod -.-> server
  prod -.-> facilitator

  extra --> protocol
  extra -.-> client
  extra -.-> server
  extra -.-> facilitator

  http -.-> protocol
  http -.-> client
  http -.-> server
  http -.-> facilitator
  http -.-> extensions

  mcp --> protocol
  mcp --> client
  mcp --> server
  mcp --> facilitator

  umbrella --> protocol
  umbrella -.-> http
  umbrella -.-> mcp
  umbrella -.-> evm
  umbrella -.-> svm
  umbrella -.-> prod
  umbrella -.-> extra
```

`r402-contract` has no r402 path dependency. SIWX is crates.io 0.6, not a workspace crate.

Publish order (crates.io): `r402-protocol` → `r402-client` → `r402-facilitator` → `r402-server` → `r402-extensions` → chain crates → `r402-http` / `r402-mcp` → `r402`. **`r402-contract` is not published.**

## Feature flags

Umbrella `default = ["evm", "http"]` opens those crates' `client` and
`server` features. Other chains: `r402` features of the same name (`svm`,
not `solana`; `avm`, not `algorand`). `full` = production chains + `http` +
`mcp` + role overlays. `tron` / `casper` exist and stay out of `full`.
`r402-contract` is never an umbrella feature.

Chain crates: `client` / `server` / `facilitator` / `telemetry` / `full`
(default empty).

| Crate | Features |
| --- | --- |
| `r402-http` | `client`, `server`, `siwx` |
| `r402-extensions` | default `ext-bazaar` / `ext-payment-id` / `ext-eip2612` / `ext-erc20-approval`; optional `siwx`, `cache`, `all-extensions` |
| `r402-facilitator` | default `http-client`; `telemetry`, `metrics` |
| `r402-mcp` | default `client` + `server`; `facilitator`, `telemetry`, `cache`, `metrics` |
| `r402-server` | `telemetry`, `metrics` |

[r402-crate]: https://img.shields.io/crates/v/r402.svg
[r402-crate-url]: https://crates.io/crates/r402
[r402-protocol-crate]: https://img.shields.io/crates/v/r402-protocol.svg
[r402-protocol-crate-url]: https://crates.io/crates/r402-protocol
[r402-client-crate]: https://img.shields.io/crates/v/r402-client.svg
[r402-client-crate-url]: https://crates.io/crates/r402-client
[r402-server-crate]: https://img.shields.io/crates/v/r402-server.svg
[r402-server-crate-url]: https://crates.io/crates/r402-server
[r402-facilitator-crate]: https://img.shields.io/crates/v/r402-facilitator.svg
[r402-facilitator-crate-url]: https://crates.io/crates/r402-facilitator
[r402-extensions-crate]: https://img.shields.io/crates/v/r402-extensions.svg
[r402-extensions-crate-url]: https://crates.io/crates/r402-extensions
[r402-evm-crate]: https://img.shields.io/crates/v/r402-evm.svg
[r402-evm-crate-url]: https://crates.io/crates/r402-evm
[r402-svm-crate]: https://img.shields.io/crates/v/r402-svm.svg
[r402-svm-crate-url]: https://crates.io/crates/r402-svm
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
[r402-avm-crate]: https://img.shields.io/crates/v/r402-avm.svg
[r402-avm-crate-url]: https://crates.io/crates/r402-avm
[r402-aptos-crate]: https://img.shields.io/crates/v/r402-aptos.svg
[r402-aptos-crate-url]: https://crates.io/crates/r402-aptos
[r402-keeta-crate]: https://img.shields.io/crates/v/r402-keeta.svg
[r402-keeta-crate-url]: https://crates.io/crates/r402-keeta
[r402-tvm-crate]: https://img.shields.io/crates/v/r402-tvm.svg
[r402-tvm-crate-url]: https://crates.io/crates/r402-tvm
[r402-stellar-crate]: https://img.shields.io/crates/v/r402-stellar.svg
[r402-stellar-crate-url]: https://crates.io/crates/r402-stellar
[r402-concordium-crate]: https://img.shields.io/crates/v/r402-concordium.svg
[r402-concordium-crate-url]: https://crates.io/crates/r402-concordium
[r402-http-crate]: https://img.shields.io/crates/v/r402-http.svg
[r402-http-crate-url]: https://crates.io/crates/r402-http
[r402-mcp-crate]: https://img.shields.io/crates/v/r402-mcp.svg
[r402-mcp-crate-url]: https://crates.io/crates/r402-mcp
[r402-doc]: https://img.shields.io/docsrs/r402.svg
[r402-doc-url]: https://docs.rs/r402
[r402-protocol-doc]: https://img.shields.io/docsrs/r402-protocol.svg
[r402-protocol-doc-url]: https://docs.rs/r402-protocol
[r402-client-doc]: https://img.shields.io/docsrs/r402-client.svg
[r402-client-doc-url]: https://docs.rs/r402-client
[r402-server-doc]: https://img.shields.io/docsrs/r402-server.svg
[r402-server-doc-url]: https://docs.rs/r402-server
[r402-facilitator-doc]: https://img.shields.io/docsrs/r402-facilitator.svg
[r402-facilitator-doc-url]: https://docs.rs/r402-facilitator
[r402-extensions-doc]: https://img.shields.io/docsrs/r402-extensions.svg
[r402-extensions-doc-url]: https://docs.rs/r402-extensions
[r402-evm-doc]: https://img.shields.io/docsrs/r402-evm.svg
[r402-evm-doc-url]: https://docs.rs/r402-evm
[r402-svm-doc]: https://img.shields.io/docsrs/r402-svm.svg
[r402-svm-doc-url]: https://docs.rs/r402-svm
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
[r402-avm-doc]: https://img.shields.io/docsrs/r402-avm.svg
[r402-avm-doc-url]: https://docs.rs/r402-avm
[r402-aptos-doc]: https://img.shields.io/docsrs/r402-aptos.svg
[r402-aptos-doc-url]: https://docs.rs/r402-aptos
[r402-keeta-doc]: https://img.shields.io/docsrs/r402-keeta.svg
[r402-keeta-doc-url]: https://docs.rs/r402-keeta
[r402-tvm-doc]: https://img.shields.io/docsrs/r402-tvm.svg
[r402-tvm-doc-url]: https://docs.rs/r402-tvm
[r402-stellar-doc]: https://img.shields.io/docsrs/r402-stellar.svg
[r402-stellar-doc-url]: https://docs.rs/r402-stellar
[r402-concordium-doc]: https://img.shields.io/docsrs/r402-concordium.svg
[r402-concordium-doc-url]: https://docs.rs/r402-concordium
[r402-http-doc]: https://img.shields.io/docsrs/r402-http.svg
[r402-http-doc-url]: https://docs.rs/r402-http
[r402-mcp-doc]: https://img.shields.io/docsrs/r402-mcp.svg
[r402-mcp-doc-url]: https://docs.rs/r402-mcp
