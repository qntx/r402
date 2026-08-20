# r402-stellar

Stellar chain support for the [x402 payment protocol][x402], part of the
[`r402`](../r402) workspace.

[x402]: https://www.x402.org

## Highlights

- **Exact scheme** — one SEP-41 `transfer` authorized as Soroban auth entries.
  The facilitator is the transaction source, pays fees, and optionally wraps a
  fee-bump.
- **CAIP-2 networks** — `stellar:pubnet` and `stellar:testnet`. Pubnet requires
  an operator RPC URL.
- **Sponsored fees** — `price_tag` copies `areFeesSponsored: true` from
  `/supported`. Clients must set the same extra; `false` is rejected.
- **In-process facilitator** — `stellar-rpc-client` 27 verify, rebuild, submit,
  and confirm. Duplicate settle is blocked by `SettlementCache` (120s TTL)
  keyed by the payload transaction XDR.

## Cargo Features

| Feature       | Surface                                              |
| ------------- | ---------------------------------------------------- |
| `client`      | Buyer-side auth-entry signing.                       |
| `server`      | Server-side `PriceTag` generation.                   |
| `facilitator` | On-chain verify and settle (Soroban RPC).            |
| `telemetry`   | `tracing` instrumentation.                           |
| `full`        | All of the above.                                    |

## Quick Start (server price tag)

```rust,ignore
use r402_stellar::{StellarExact, USDC};

let tag = StellarExact::price_tag(pay_to, USDC::stellar_testnet().amount(10_000_000));
```

## Documentation

- Crate docs: <https://docs.rs/r402-stellar>
- Project README: [`../../README.md`](../../README.md)

## License

Dual-licensed under [MIT](../../LICENSE-MIT) and
[Apache-2.0](../../LICENSE-APACHE).
