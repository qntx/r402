# r402-core

Core types, traits, and wire formats for the x402 payment protocol.

This crate is blockchain-agnostic. It defines the shared data model used by
client, server, and facilitator implementations across every chain. Chain
specific behavior lives in sibling crates (`r402-evm`, `r402-svm`).

## Modules

- [`wire`] — On-the-wire JSON types (versioned: `wire::v2`)
- [`facilitator`] — `Facilitator` trait and `DynFacilitator` erasure
- [`scheme`] — Payment scheme registry and `sealed` builder traits
- [`hooks`] — Lifecycle hooks for `verify`/`settle`
- [`extensions`] — Extension framework for x402 protocol extensions
- [`payment`] — `Payment<State>` typestate for compile-time lifecycle safety
- [`chain`] — CAIP-2 chain identifiers, address, provider abstractions
- [`amount`] — Human-readable currency amount parsing (`"$0.01"` etc.)
- [`error`] — Layered error types (`FacilitatorError`, `VerificationError`, ...)

## Feature Flags

| Flag             | Default | Effect                                     |
| ---------------- | :-----: | ------------------------------------------ |
| `telemetry`      |         | Enable `tracing` instrumentation           |
| `cache`          |         | Enable `moka`-backed TTL caches            |
| `ext-bazaar`     |         | Enable the bazaar resource discovery ext.  |
| `ext-payment-id` |         | Enable the payment-identifier idempotency. |
| `all-extensions` |         | Enable every built-in extension            |
| `full`           |         | Enable every feature                       |

## License

Licensed under either of MIT or Apache-2.0, at your option.
