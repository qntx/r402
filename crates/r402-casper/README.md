# r402-casper

Casper Network support for the [x402 payment protocol][x402], part of the
[`r402`](../r402) workspace.

[x402]: https://www.x402.org

## Scope

This crate is a **remote-facilitator** integration:

- Local preflight validates wire shape, accepted↔requirements consistency,
  mote amounts, timing, and `publicKey` → `authorization.from` binding.
- **Cryptographic signature verification and on-chain settlement** are
  performed by a hosted (or self-hosted) Casper x402 facilitator — default
  `https://x402-facilitator.cspr.cloud`.
- There is no in-process Casper RPC / key-custody stack. That keeps the
  dependency footprint comparable to other chain crates while matching the
  Casper Association deployment model.

## Highlights

- **Exact scheme** — CEP-18 / CEP-3009 `transfer_with_authorization` via
  pre-signed EIP-712 authorizations (wCSPR on testnet built in).
- **CAIP-2 networks** — `casper:casper` (mainnet) and `casper:casper-test`.
- **Exact mote arithmetic** — 9 decimals; sub-mote precision is a typed
  error, never a silent truncation.
- **Facilitator client** — `/verify`, `/settle`, `/supported` with optional
  local preflight and configurable timeout.

## Cargo Features

| Feature        | Surface                                              |
| -------------- | ---------------------------------------------------- |
| `client`       | `CasperExactClient` (`SchemeClient`) + EIP-712 digest. |
| `server`       | Server-side `PriceTag` generation.                   |
| `facilitator`  | Remote facilitator client (`verify` / `settle`).     |
| `http-client`  | Default `reqwest` transport (`ReqwestTransport`).    |
| `telemetry`    | `tracing` instrumentation.                           |
| `full`         | All of the above.                                    |

### Client (`SchemeClient`)

```rust,ignore
use r402_casper::{CasperExactClient, CasperSigner};
use r402_http::client::X402Client;

// `MyWallet: CasperSigner` supplies public_key + sign_digest.
let x402 = X402Client::new().register(CasperExactClient::new(my_wallet));
```

### Facilitator (`http-client`)

```rust,ignore
use r402_casper::CasperExactFacilitator;

let facilitator = CasperExactFacilitator::hosted_http();
// or: CasperExactFacilitator::from_env_http()?;
```

## Quick Start (server price tag)

```rust
use r402_casper::chain::Address;
use r402_casper::{CasperExact, WCSPR};

let pay_to: Address = "00fedcba0987654321fedcba0987654321fedcba0987654321fedcba0987654321"
    .parse()
    .unwrap();
let tag = CasperExact::price_tag(pay_to, WCSPR::casper_test().amount(1_500_000_000));
assert_eq!(tag.requirements.network.to_string(), "casper:casper-test");
```

Override the facilitator URL with `R402_CASPER_FACILITATOR_URL`.

## References

- Scheme spec: [`scheme_exact_casper.md`](../../vendor/x402/specs/schemes/exact/scheme_exact_casper.md)
- Casper x402 reference: <https://github.com/make-software/casper-x402>
- Facilitator docs: <https://docs.cspr.cloud>

## License

Dual-licensed under [MIT](../../LICENSE-MIT) and
[Apache-2.0](../../LICENSE-APACHE).
