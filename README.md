# r402

An unofficial Rust implementation of the [x402 Payment Protocol](https://www.x402.org/), designed as a learning reference for understanding how the x402 protocol works in a Rust ecosystem.

> **⚠️ Important Notice**
>
> This project is developed **strictly for educational and learning purposes**. It is **NOT recommended for use in production environments**.
>
> The architecture and design of this project are entirely based on the official [Coinbase x402 protocol specification](https://www.x402.org/) and its reference implementations. For production-ready usage, please refer to the more mature and actively maintained community implementation:  [`x402-rs/x402-rs`](https://github.com/x402-rs/x402-rs) — The recommended Rust SDK for the x402 Payment Protocol.
>

## Overview

r402 provides a modular Rust SDK for the x402 payment protocol, covering the full lifecycle of HTTP 402-based micropayments including client payment signing, server payment gating, and facilitator verification/settlement.

### Crates

| Crate | Description |
| ------- | ------------- |
| `r402` | Core library — protocol types, scheme traits, client/server/facilitator abstractions, and hook system |
| `r402-evm` | EVM (EIP-155) chain support — ERC-3009 transfer authorization, multi-signer management, nonce tracking |
| `r402-svm` | Solana (SVM) chain support (WIP) |
| `r402-http` | HTTP transport layer — Axum payment gate middleware, reqwest client middleware, facilitator HTTP client |
| `r402-facilitator` | Standalone facilitator binary |

### Key Features

- **V1 & V2 Protocol Support** — Compatible with both legacy and current x402 wire formats
- **Scheme Abstraction** — Pluggable `SchemeClient`, `SchemeServer`, and `SchemeFacilitator` traits for extensibility
- **Hook Lifecycle** — Comprehensive before/after/failure hooks for client, server, and facilitator operations
- **EVM Chain Provider** — Round-robin multi-signer, pending nonce management, EIP-1559 gas pricing
- **HTTP Middleware** — Tower-based payment gate (server) and reqwest middleware (client) for seamless integration
- **dyn-compatible Async** — Uses `BoxFuture` for trait object safety without relying on `async-trait` macro

## Acknowledgments

This project draws heavily from the following resources:

- [x402 Protocol Specification](https://www.x402.org/) by Coinbase
- [coinbase/x402](https://github.com/coinbase/x402) — Official TypeScript/Python reference implementations
- [x402-rs/x402-rs](https://github.com/x402-rs/x402-rs) — Community Rust implementation

## License

This project is licensed under either of the following licenses, at your option:

- Apache License, Version 2.0, ([LICENSE-APACHE](LICENSE-APACHE) or [https://www.apache.org/licenses/LICENSE-2.0](https://www.apache.org/licenses/LICENSE-2.0))
- MIT license ([LICENSE-MIT](LICENSE-MIT) or [https://opensource.org/licenses/MIT](https://opensource.org/licenses/MIT))

Unless you explicitly state otherwise, any contribution intentionally submitted for inclusion in this project by you, as defined in the Apache-2.0 license, shall be dually licensed as above, without any additional terms or conditions.
