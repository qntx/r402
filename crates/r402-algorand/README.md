# r402-algorand

Algorand chain support for the [x402 payment protocol][x402], part of the
[`r402`](../r402) workspace.

[x402]: https://www.x402.org

## Highlights

- **Exact scheme** — an atomic group with an ASA transfer to `payTo` and an
  optional 0-amount fee-payer payment. The facilitator signs the fee-payer
  transaction, simulates, broadcasts, and waits for confirmation.
- **CAIP-2 networks** — `algorand:wGHE2Pwdvd7S12BL5FaOP20EGYesN73k`
  (mainnet) and `algorand:SGO1GKSzyE7IEPItTxCByw9x8FmnrCDe` (testnet).
  On-chain `gh` is always the full genesis hash, not the truncated CAIP-2
  reference.
- **algod REST** — suggested params, simulate, broadcast, and pending
  confirmation against AlgoNode (overridable). Transactions use canonical
  msgpack (sorted keys, default values omitted).
- **Fee cap** — fee-payer fee must be `≤ 5000 µAlgo × groupSize`.

## Cargo Features

| Feature       | Surface                                              |
| ------------- | ---------------------------------------------------- |
| `client`      | Buyer-side atomic group construction and signing.    |
| `server`      | Server-side `PriceTag` generation (`feePayer` extra).|
| `facilitator` | On-chain verify and settle (algod REST).             |
| `telemetry`   | `tracing` instrumentation.                           |
| `full`        | All of the above.                                    |

## Quick Start (server price tag)

```rust,ignore
use r402_algorand::{AlgorandExact, USDC};

let tag = AlgorandExact::price_tag(pay_to, USDC::algorand_testnet().amount(1_000_000u64));
```

## Documentation

- Crate docs: <https://docs.rs/r402-algorand>
- Project README: [`../../README.md`](../../README.md)

## License

Dual-licensed under [MIT](../../LICENSE-MIT) and
[Apache-2.0](../../LICENSE-APACHE).
