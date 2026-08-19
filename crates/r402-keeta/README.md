# r402-keeta

Keeta chain support for the [x402 payment protocol][x402], part of the
[`r402`](../r402) workspace.

[x402]: https://www.x402.org

## Highlights

- **Exact scheme** — the buyer signs one `SEND` block (ASN.1 DER). The
  facilitator sponsors network fees via `TransmitOptions::with_fee_signer`
  and cannot rewrite the signed transfer.
- **CAIP-2 networks** — `keeta:21378` (mainnet) and `keeta:1413829460`
  (testnet).
- **No client extra from `/supported`** — `getExtra` is undefined.
  `price_tag` sets `enricher: None`. Optional `extra.external` is a
  client-only `SEND` field when the seller sets it on requirements.
  `supported().signers["keeta:*"]` lists fee-payer accounts.
- **In-process facilitator** — `Block::try_from` verifies DER + signature.
  Settle is serialized per fee-payer (`mpsc`) and parallel across payers.

## Cargo Features

| Feature       | Surface                                              |
| ------------- | ---------------------------------------------------- |
| `client`      | Buyer-side `UserClient::init_builder` signing.       |
| `server`      | Server-side `PriceTag` generation.                   |
| `facilitator` | On-chain verify and settle (`keetanetwork-client`).  |
| `telemetry`   | `tracing` instrumentation.                           |
| `full`        | All of the above.                                    |

## Quick Start (server price tag)

```rust,ignore
use r402_keeta::{KeetaExact, USDC};

let tag = KeetaExact::price_tag(pay_to, USDC::keeta_testnet().amount(1_000_000u128));
```

## Documentation

- Crate docs: <https://docs.rs/r402-keeta>
- Project README: [`../../README.md`](../../README.md)

## License

Dual-licensed under [MIT](../../LICENSE-MIT) and
[Apache-2.0](../../LICENSE-APACHE).
