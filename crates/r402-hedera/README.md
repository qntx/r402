# r402-hedera

Hedera chain support for the [x402 payment protocol][x402], part of the
[`r402`](../r402) workspace.

[x402]: https://www.x402.org

## Highlights

- **Exact scheme** — a payer-signed `TransferTransaction` (HBAR or HTS FT)
  with the facilitator as `feePayer`. The facilitator adds its signature
  and submits; success requires a `SUCCESS` receipt.
- **CAIP-2 networks** — `hedera:mainnet` and `hedera:testnet`.
- **`extra.feePayer` only** — copied onto `PriceTag` from `/supported`.
  `aliasPolicy` is facilitator config (default `reject`), not wire extra.
- **In-process facilitator** — Mirror Node REST preflight (balance and
  token association) plus Hiero SDK execute. Duplicate settle is blocked
  by `SettlementCache` keyed by the base64 transaction.

## Cargo Features

| Feature       | Surface                                                |
| ------------- | ------------------------------------------------------ |
| `client`      | Buyer-side partially signed `TransferTransaction`.     |
| `server`      | Server-side `PriceTag` generation.                     |
| `facilitator` | On-chain verify and settle (Mirror REST + Hiero SDK).  |
| `telemetry`   | `tracing` instrumentation.                             |
| `full`        | All of the above.                                      |

## Quick Start (server price tag)

```rust,ignore
use r402_hedera::{HederaExact, USDC};

let tag = HederaExact::price_tag(pay_to, USDC::hedera_testnet().amount(1_000_000u64));
```

## Documentation

- Crate docs: <https://docs.rs/r402-hedera>
- Project README: [`../../README.md`](../../README.md)

## License

Dual-licensed under [MIT](../../LICENSE-MIT) and
[Apache-2.0](../../LICENSE-APACHE).
