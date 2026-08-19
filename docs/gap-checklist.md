# Chain-parity gap checklist

Status of official TS mechanism families versus r402 crates.

| Family | TS package | r402 crate | Status |
| --- | --- | --- | --- |
| EVM | `@x402/evm` | `r402-evm` | Parity (exact, upto, auth-capture, batch-settlement) |
| SVM | `@x402/svm` | `r402-solana` | exact shipped; upto deferred (escrow, not HTTP Settlement-Overrides) |
| NEAR | `@x402/near` | `r402-near` | exact E2E shipped |
| TVM (TON) | `@x402/tvm` | `r402-tvm` | exact E2E shipped |
| Aptos | `@x402/aptos` | — | Missing (after MSRV 1.95) |
| Aptos | `@x402/aptos` | `r402-aptos` | exact E2E shipped |
| Algorand | `@x402/avm` | `r402-algorand` | exact E2E shipped |
| Hedera | `@x402/hedera` | `r402-hedera` | exact E2E shipped |
| Keeta | `@x402/keeta` | `r402-keeta` | exact E2E shipped |
| Stellar | `@x402/stellar` | `r402-stellar` | exact E2E shipped |
| XRPL | `@x402/xrpl` | `r402-xrpl` | exact E2E shipped |
| Concordium | `@x402/concordium` | — | Deferred (no SPDX-clean SDK) |
| Tron | — | `r402-tron` | r402-only |
| Casper | spec only | `r402-casper` | r402-only (remote facilitator) |

Deferred items are not covered by other work:

- Concordium stays out until an SPDX-clean SDK exists.
- SVM `upto` is on-chain escrow in `@x402/svm`. HTTP `Settlement-Overrides` is EVM/upto billing and does not implement that escrow.
