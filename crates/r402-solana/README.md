# r402-solana

Solana chain support for the [x402 payment protocol][x402], part of the
[`r402`](../r402) workspace.

[x402]: https://www.x402.org

## Highlights

- **Exact scheme** — SPL Token `TransferChecked` with pre-signed
  authorisation, fee-payer-bound facilitator submission, and atomic
  duplicate-settlement guard via `SettlementCache`.
- **Amount rule** — matches `scheme_exact_svm.md` §1.4 and the Go/TS
  facilitators: on-chain amount must be **`>=` required** (overpayment
  allowed; underpayment rejected). Settle reports the **actual**
  transfer amount.
- **Wallet adapters** — Phantom Lighthouse, Solflare Lighthouse, and the
  SPL Memo program are pre-allowlisted so on-chain wallet integrations
  work out of the box.
- **Compute-budget verification** — clients must declare a compute-unit
  limit and price; r402 enforces a recommended cap of
  `SOLANA_MAX_COMPUTE_UNIT_PRICE_MICROLAMPORTS` (5,000,000 µLAM/CU)
  aligned with the Go SDK.
- **Three-layer fee-payer defence** — buyer-signed `extra.feePayer`,
  signer-set membership, and an unconditional fee-payer-not-in-instructions
  invariant.

## Cargo Features

| Feature        | Surface                                           |
| -------------- | ------------------------------------------------- |
| `client`       | Client-side transaction signing and serialisation.|
| `server`       | Server-side `PriceTag` generation.                |
| `facilitator`  | Facilitator-side verify and settle.               |
| `telemetry`    | OpenTelemetry tracing instrumentation.            |
| `full`         | All of the above.                                 |

## Documentation

- Crate docs: <https://docs.rs/r402-solana>
- Project README: [`../README.md`](../README.md).

## Smart-wallet Path 2

Opt in via `SolanaExactFacilitatorConfig::with_smart_wallet_verification(true)`
(default **off**, matching foundation TS). When enabled, Path 1 layout
failures fall through to outcome-based `TransferChecked` matching
(`scheme_exact_svm.md` §3.2).

**Current coverage:** top-level multi-ix TransferChecked + successful
simulation with `inner_instructions: true`. **Follow-up:** plumb
`RpcSimulateTransactionResult.inner_instructions` through
`SolanaChainProviderLike` for CPI-only wallets (Squads/Swig).

## License

Dual-licensed under [MIT](../LICENSE-MIT) and
[Apache-2.0](../LICENSE-APACHE).
