# r402-casper

**Extra Casper chain support (not an x402 protocol mechanism).**

**Remote-facilitator client.** Local `validate_request` is shape and timing
preflight only. Cryptographic verification and on-chain settlement stay on
the remote facilitator (`R402_CASPER_FACILITATOR_URL`, default
`https://x402-facilitator.cspr.cloud`).

## Highlights

- **Exact scheme** — CEP-18 `transfer_with_authorization` via a pre-signed
  EIP-712 authorization.
- **CAIP-2 networks** — `casper:casper` and `casper:casper-test`.
- **Remote facilitator** — `CasperExactFacilitator::try_new` talks
  `/verify`, `/settle`, and `/supported`. Crypto and chain stay remote.

## Cargo Features

| Feature       | Surface                                              |
| ------------- | ---------------------------------------------------- |
| `client`      | Buyer-side EIP-712 signing.                          |
| `server`      | Server-side `PriceTag` generation.                   |
| `facilitator` | Remote facilitator HTTP client.                      |
| `telemetry`   | `tracing` instrumentation.                           |
| `full`        | All of the above.                                    |

## Quick Start (server price tag)

```rust,ignore
use r402_casper::{CasperExact, WCSPR};

let tag = CasperExact::price_tag(pay_to, WCSPR::casper_test().amount(1_000_000_000));
```

## License

Dual-licensed under [MIT](../../LICENSE-MIT) and
[Apache-2.0](../../LICENSE-APACHE).
