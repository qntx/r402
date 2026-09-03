# r402-tron

**Experimental / not an x402 protocol mechanism.** Not re-exported from
umbrella `r402` (no `tron` feature).

Tron chain support, part of the [`r402`](../r402) workspace.

## Highlights

- **Exact scheme** — EIP-3009 `transferWithAuthorization` (TIP-712
  signing) plus SUN.io Permit2 via `x402ExactPermit2Proxy` for tokens
  without EIP-3009 (e.g. USDT).
- **CAIP-2 networks** — mainnet (`tron:0x2b6653dc`) and Nile (`tron:0xcd8690dc`).
- **Base58Check addresses** — native `T…` form with the `0x41` prefix
  handled at the type boundary.
- **On-chain facilitator** — verify and settle through the TronGrid HTTP
  API (no JSON-RPC). `TronExactFacilitator::try_new(provider)`.

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
use alloy_primitives::U256;
use r402_tron::{AssetTransferMethod, TronExact, USDT};

let tag = TronExact::price_tag(
    pay_to,
    &USDT::nile().amount(U256::from(1_000_000u64)),
    Some(AssetTransferMethod::Permit2),
);
```

## License

Dual-licensed under [MIT](../../LICENSE-MIT) and
[Apache-2.0](../../LICENSE-APACHE).
