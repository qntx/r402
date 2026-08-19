# Chain-parity gap checklist

Status of official TS mechanism families versus r402 crates.

| Family | TS package | r402 crate | Status |
| --- | --- | --- | --- |
| EVM | `@x402/evm` | `r402-evm` | Parity (exact, upto, auth-capture, batch-settlement) |
| SVM | `@x402/svm` | `r402-solana` | exact only (upto escrow out of scope) |
| NEAR | `@x402/near` | `r402-near` | exact E2E shipped |
| TVM (TON) | `@x402/tvm` | — | Missing |
| Aptos | `@x402/aptos` | — | Missing (after MSRV 1.95) |
| Algorand | `@x402/avm` | — | Missing |
| Hedera | `@x402/hedera` | `r402-hedera` | exact E2E shipped |
| Algorand | `@x402/avm` | `r402-algorand` | exact E2E shipped |
| Hedera | `@x402/hedera` | — | Missing |
| Keeta | `@x402/keeta` | — | Missing |
| Stellar | `@x402/stellar` | — | Missing (after MSRV 1.95) |
| XRPL | `@x402/xrpl` | `r402-xrpl` | exact E2E shipped |
| Concordium | `@x402/concordium` | — | Deferred (no SPDX-clean SDK) |
| Tron | — | `r402-tron` | r402-only |
| Casper | spec only | `r402-casper` | r402-only (remote facilitator) |
