# r402-tvm

TON (TVM) chain support for the [x402 payment protocol][x402], part of the
[`r402`](../r402) workspace.

[x402]: https://www.x402.org

## Highlights

- **Exact scheme** — one TEP-74 `jetton_transfer` authorized as a W5R1
  `internal_signed` message. The facilitator Highload V3 wallet relays the
  signed body and sponsors gas.
- **CAIP-2 networks** — `tvm:-239` (mainnet) and `tvm:-3` (testnet). The
  signed integer reference is preserved through [`ChainId`][chain-id].
- **Sponsored fees** — `price_tag` copies `areFeesSponsored: true` from
  `/supported`. Clients must set the same extra.
- **In-process facilitator** — Toncenter / TonAPI REST verify and settle.
  Duplicate settle is blocked by `SettlementCache` (120s TTL) keyed by the
  settlement BoC hash. Failed and successful sends keep the key until TTL.

[chain-id]: https://docs.rs/r402-core/latest/r402_core/chain/struct.ChainId.html

## Cargo Features

| Feature       | Surface                                              |
| ------------- | ---------------------------------------------------- |
| `client`      | Buyer-side W5R1 settlement BoC signing.              |
| `server`      | Server-side `PriceTag` generation.                   |
| `facilitator` | On-chain verify and Highload V3 batch settle.        |
| `telemetry`   | `tracing` instrumentation.                           |
| `full`        | All of the above.                                    |

## Quick Start (server price tag)

```rust,ignore
use r402_tvm::{TvmExact, USDT};

let tag = TvmExact::price_tag(pay_to, USDT::tvm_testnet().amount(10_000u128));
```

## Documentation

- Crate docs: <https://docs.rs/r402-tvm>
- Project README: [`../../README.md`](../../README.md)

## License

Dual-licensed under [MIT](../../LICENSE-MIT) and
[Apache-2.0](../../LICENSE-APACHE).
