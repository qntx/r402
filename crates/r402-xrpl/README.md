# r402-xrpl

XRP Ledger support for the [x402 payment protocol][x402], part of the
[`r402`](../r402) workspace.

[x402]: https://www.x402.org

## Highlights

- **Exact scheme** — the buyer signs an XRPL `Payment` transaction; the
  facilitator submits the signed blob. The payer pays the network fee
  (`areFeesSponsored: false`).
- **CAIP-2 networks** — `xrpl:0` (mainnet), `xrpl:1` (testnet), `xrpl:2`
  (devnet).
- **XRP and RLUSD** — native drops or issued-currency decimal values.
  `price_tag` copies `areFeesSponsored: false` (and `issuer` for IOU).
- **Sequence or ticket** — `extra.assetTransferMethod` selects account
  `Sequence` or a pre-created `TicketSequence`.
- **In-process facilitator** — verify and settle against JSON-RPC HTTP
  (overridable). Duplicate settle is blocked by `SettlementCache` keyed
  by transaction hash (TTL floor 120s). `supported().signers["xrpl:*"]`
  is empty.

## Cargo Features

| Feature       | Surface                                              |
| ------------- | ---------------------------------------------------- |
| `client`      | Buyer-side `Payment` signing.                        |
| `server`      | Server-side `PriceTag` generation.                   |
| `facilitator` | On-chain verify and settle (JSON-RPC).               |
| `telemetry`   | `tracing` instrumentation.                           |
| `full`        | All of the above.                                    |

## Quick Start (server price tag)

```rust,ignore
use r402_xrpl::{XrplExact, XRP};

let tag = XrplExact::price_tag(pay_to, XRP::testnet().amount(1_000_000u64));
```

## Documentation

- Crate docs: <https://docs.rs/r402-xrpl>
- Project README: [`../../README.md`](../../README.md)

## License

Dual-licensed under [MIT](../../LICENSE-MIT) and
[Apache-2.0](../../LICENSE-APACHE).
