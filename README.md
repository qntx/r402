# r402

This repository is based on [x402-rs/x402-rs](https://github.com/x402-rs/x402-rs) and includes qntx-specific patches and extensions to better integrate with our internal projects. If you're looking for the canonical Rust implementation of x402, please use the upstream project directly.

## Overview

r402 provides a modular Rust SDK for the x402 payment protocol, covering the full lifecycle of HTTP 402-based micropayments — client payment signing, server payment gating, and facilitator verification/settlement.

### Crates

| Crate | Description |
| --- | --- |
| `r402` | Core library — protocol types, scheme traits, client/server/facilitator abstractions, and hook system |
| `r402-evm` | EVM (EIP-155) chain support — ERC-3009 transfer authorization, multi-signer management, nonce tracking |
| `r402-svm` | Solana (SVM) chain support (WIP) |
| `r402-http` | HTTP transport layer — Axum payment gate middleware, reqwest client middleware, facilitator HTTP client |

## Acknowledgments

- [x402-rs/x402-rs](https://github.com/x402-rs/x402-rs) — Upstream Rust implementation (community)
- [x402 Protocol Specification](https://www.x402.org/) by Coinbase
- [coinbase/x402](https://github.com/coinbase/x402) — Official TypeScript/Python reference implementations

## License

This project is licensed under either of the following licenses, at your option:

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or <https://www.apache.org/licenses/LICENSE-2.0>)
- MIT License ([LICENSE-MIT](LICENSE-MIT) or <https://opensource.org/licenses/MIT>)

Unless you explicitly state otherwise, any contribution intentionally submitted for inclusion in this project by you, as defined in the Apache-2.0 license, shall be dually licensed as above, without any additional terms or conditions.
