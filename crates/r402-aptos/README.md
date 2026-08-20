# r402-aptos

Aptos chain support for the [x402 payment protocol][x402], part of the
[`r402`](../r402) workspace.

[x402]: https://www.x402.org

## Highlights

- **Exact scheme** — a sender-signed `primary_fungible_store::transfer` of a
  fungible asset (USDC by default). The facilitator is the optional
  `feePayer`: it verifies the payload, adds its signature, and submits.
- **CAIP-2 networks** — `aptos:1` (mainnet) and `aptos:2` (testnet).
- **TS payload encoding** — outer base64 of UTF-8 JSON
  `{ transaction: [u8], senderAuthenticator: [u8] }`. Inner bytes are
  BCS from `aptos-sdk` 0.6.0 (`SimpleTransaction` + `AccountAuthenticator`).
- **Gas caps** — sponsored txs reject `max_gas_amount > 500_000` and
  `gas_unit_price > 1000`.
- **In-process facilitator** — `aptos-sdk` verify (signature, simulate,
  FA balance) and settle. Duplicate settle is blocked by `SettlementCache`
  keyed by the base64 payload.

## Cargo Features

| Feature       | Surface                                                         |
| ------------- | --------------------------------------------------------------- |
| `client`      | Buyer-side sponsored `primary_fungible_store::transfer` signing. |
| `server`      | Server-side `PriceTag` generation.                              |
| `facilitator` | On-chain verify and settle via `aptos-sdk` 0.6.                 |
| `telemetry`   | `tracing` instrumentation.                                      |
| `full`        | All of the above.                                               |

## Quick Start (server price tag)

```rust,ignore
use r402_aptos::{AptosExact, USDC};

let tag = AptosExact::price_tag(pay_to, USDC::aptos_testnet().amount(1_000_000u64));
```

## Documentation

- Crate docs: <https://docs.rs/r402-aptos>
- Project README: [`../../README.md`](../../README.md)

## License

Dual-licensed under [MIT](../../LICENSE-MIT) and
[Apache-2.0](../../LICENSE-APACHE).
