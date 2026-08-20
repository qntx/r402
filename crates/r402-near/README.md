# r402-near

NEAR chain support for the [x402 payment protocol][x402], part of the
[`r402`](../r402) workspace.

[x402]: https://www.x402.org

## Highlights

- **Exact scheme** — one NEP-141 `ft_transfer` authorized as a NEP-366
  `SignedDelegate`. The facilitator relayer submits the outer transaction
  and reports success only when the inner `ft_transfer` receipt succeeds.
- **CAIP-2 networks** — `near:mainnet` and `near:testnet`.
- **No client extra** — the relayer is facilitator-local. `price_tag`
  sets `enricher: None`. `supported().signers["near:*"]` lists relayer
  account IDs.
- **In-process facilitator** — verify and settle against FastNEAR JSON-RPC
  (overridable). Duplicate settle is blocked by `SettlementCache` (120s TTL)
  keyed by the base64 delegate.

## Cargo Features

| Feature       | Surface                                              |
| ------------- | ---------------------------------------------------- |
| `client`      | Buyer-side NEP-366 delegate signing.                 |
| `server`      | Server-side `PriceTag` generation.                   |
| `facilitator` | On-chain verify and settle (JSON-RPC).               |
| `telemetry`   | `tracing` instrumentation.                           |
| `full`        | All of the above.                                    |

## Quick Start (server price tag)

```rust,ignore
use r402_near::{NearExact, USDC};

let tag = NearExact::price_tag(pay_to, USDC::near_testnet().amount(1_000_000u128));
```

## Documentation

- Crate docs: <https://docs.rs/r402-near>
- Project README: [`../../README.md`](../../README.md)

## License

Dual-licensed under [MIT](../../LICENSE-MIT) and
[Apache-2.0](../../LICENSE-APACHE).
