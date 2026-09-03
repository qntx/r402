# `r402-tvm`

TON (TVM) chain support for the [x402 payment protocol][x402], part of the
[`r402`](../r402) workspace.

[x402]: https://www.x402.org

Exact scheme: one TEP-74 `jetton_transfer` authorized as a W5R1
`internal_signed` message. The facilitator Highload V3 wallet relays the
signed body and sponsors gas.

CAIP-2 networks: `tvm:-239` (mainnet) and `tvm:-3` (testnet).
`price_tag` copies `areFeesSponsored: true` from `/supported`.

## Cargo Features

| Feature       | Surface                                              |
| ------------- | ---------------------------------------------------- |
| `client`      | Buyer-side W5R1 settlement BoC signing.              |
| `server`      | Server-side `PriceTag` generation.                   |
| `facilitator` | On-chain verify and Highload V3 batch settle.        |
| `telemetry`   | `tracing` instrumentation.                           |
| `full`        | All of the above.                                    |

```rust,ignore
use r402_tvm::{TvmExact, USDT};

let tag = TvmExact::price_tag(pay_to, USDT::tvm_testnet().amount(10_000u128));
```

Dual-licensed under MIT and Apache-2.0.
