# r402-core

Core types, traits, and wire formats for the x402 payment protocol.

This crate is blockchain-agnostic. It defines the shared data model used by
client, server, and facilitator implementations across every chain. Chain
specific behavior lives in sibling crates — see [`crates/README.md`](../README.md).

## Modules

- [`wire`] — On-the-wire JSON types (offer, facilitator RPC, codecs) and settlement overrides
- [`facilitator`] — `Facilitator` trait, `DynFacilitator` erasure, verify/settle hooks
- [`scheme`] — Payment scheme registry and builder traits
- [`extensions`] — Extension framework for x402 protocol extensions
- [`client`] — `PaymentClient` orchestration
- [`resource_server`] — Transport-agnostic verify → execute → settle
- [`chain`] — CAIP-2 chain identifiers, address, provider abstractions
- [`amount`] — Human-readable currency amount parsing (`"$0.01"` etc.)
- [`error`] — Layered errors and machine-readable [`ErrorReason`] wire codes

## Feature Flags

| Flag             | Default | Effect                                     |
| ---------------- | :-----: | ------------------------------------------ |
| `telemetry`      |         | Enable `tracing` instrumentation           |
| `cache`          |         | Enable `moka`-backed TTL caches            |
| `ext-bazaar`     |         | Enable the bazaar resource discovery ext.  |
| `ext-payment-id` |         | Enable the payment-identifier idempotency. |
| `ext-eip2612`    |         | Enable EIP-2612 gas-sponsoring extra.      |
| `ext-erc20-approval` |     | Enable ERC-20 approval gas-sponsoring extra. |
| `ext-agent-identity` |     | Enable the ERC-8004 agent-identity ext.    |
| `metrics`        |         | Enable the `metrics` facade counters.      |
| `all-extensions` |         | Enable every built-in extension            |
| `full`           |         | Enable every feature                       |

## License

Licensed under either of MIT or Apache-2.0, at your option.
