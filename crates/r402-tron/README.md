# r402-tron

Tron chain support for the [x402 payment protocol][x402], part of the
[`r402`](../r402) workspace.

[x402]: https://www.x402.org

## Highlights

- **Exact scheme** — EIP-3009 `transferWithAuthorization` (TIP-712
  signing) plus SUN.io Permit2 via `x402ExactPermit2Proxy` for tokens
  without EIP-3009 (e.g. USDT).
- **CAIP-2 networks** — mainnet / Nile / Shasta (`tron:0x2b6653dc`, …).
- **Base58Check addresses** — native `T…` form with the `0x41` prefix
  handled at the type boundary.
- **On-chain facilitator** — verify and settle through the TronGrid HTTP
  API (no JSON-RPC), with multi-signer support.

## Cargo Features

| Feature       | Surface                                              |
| ------------- | ---------------------------------------------------- |
| `client`      | Buyer-side EIP-3009 / Permit2 signing.               |
| `server`      | Server-side `PriceTag` generation.                   |
| `facilitator` | On-chain verify and settle (TronGrid).               |
| `telemetry`   | `tracing` instrumentation.                           |
| `full`        | All of the above.                                    |

## Quick Start (server price tag)

```rust,ignore
use r402_tron::{TronExact, USDC};

let tag = TronExact::price_tag(pay_to, USDC::nile().amount(1_000_000u64));
```

## Documentation

- Crate docs: <https://docs.rs/r402-tron>
- Project README: [`../../README.md`](../../README.md)

## License

Dual-licensed under [MIT](../../LICENSE-MIT) and
[Apache-2.0](../../LICENSE-APACHE).
