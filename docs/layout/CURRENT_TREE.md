# CURRENT_TREE — r402 as of 2026-09-03

Inventory of first-party Rust sources at ground-truth HEAD `fb0e1cdfa4990a15cf6a78f7c5ee7e5c646f9b6a` (`origin/main`, workspace **0.17.1**, tag `pre-wipe-0.17.1`). Excludes `target/`. HEAD has **no `vendor/`** (372 tree paths, 0 `vendor/`).

This file is a **behavior inventory**, not a name constraint. Old filenames/folders are not TARGET. Construction is **wipe listed crate dirs then write TARGET crates from zero** (see DESIGN.md). Re-verified 2026-09-03: `find crates -name '*.rs' | wc -l` = **295**, `wc -l` = **86 256**. Zero path/LOC mismatch vs `git ls-tree -r HEAD`.

- **295** `.rs` files, **86 256** lines (`wc -l`).
- Workspace version **0.17.1**, edition **2024**, MSRV **1.95** (aptos-sdk, not qss/ccip 1.85).
- Members: `crates/*` (16 crates). Default member: `crates/r402`.
- No `[[bin]]`, no `examples/`. Tests: `r402-core/tests/*`, `r402-evm/tests/onchain_deployment.rs`, plus in-crate `#[cfg(test)]` and several `exact/facilitator/tests.rs`.
- **19** non-`.rs` fixtures (not in the `.rs` table; FILE_MAP names them): seven `crates/r402-core/tests/fixtures/spec_v2/*.json`, `crates/r402-core/tests/wire_proptest.proptest-regressions`, and `src/exact/fixtures/*.{json,b64}` under algorand, aptos, hedera, keeta, near, stellar, tvm, xrpl.
- Matching bypass in HTTP paygate is **already closed** (`Paygate` calls `ResourceServer::find_matching_requirements`). Keep-current, not a new fix.

Audit patches (inventory facts, not TARGET):

- `SettlementMode` lives in `paygate.rs` L108–116. No `FacilitatorError::Transport`.
- `with_price_tag` → `X402Layer` (no `X402Route`, no `into_layer`).
- `Eip155Exact::price_tag` is three-arg in code; README shows two-arg (bug).
- Umbrella features are `dep:` only (do not open crate `client`/`server`).
- GrantAccess hook slot exists; no `with_auth_only`.
- `SupportedResponse.extensions` is already a JSON array.
- Metric name today: `r402_paygate_background_settle_total`.
- No Cache-Control helpers. No `EXTENSION-RESPONSES` parse.
- 700–800 LOC, should-split, **not** God: `payment_flow.rs` 779, `settlement_batcher.rs` 734, `open.rs` 713, `offer.rs` 704.

`God` = mixed concerns and/or ≥ ~800 LOC of real logic in one file (or a test bag ≥ ~800 that is not split by scenario). `Junk` = junk-drawer name or leftover bag (`hex`, `shared`, `codecs` as a crate-root peer of `chain`, duplicate `provider.rs`, `trace`).

Line counts are physical lines, including tests and comments inside the file.

---

## Crate totals

| Crate | `.rs` files | LOC | Role today |
| --- | ---: | ---: | --- |
| `r402` | 1 | 48 | Umbrella re-export |
| `r402-core` | 30 | 14 042 | Protocol + client + server + facilitator + extensions + tests |
| `r402-evm` | 50 | 11 551 | EIP-155 exact / upto / auth-capture / batch-settlement |
| `r402-solana` | 34 | 8 373 | Solana exact + upto payment-channel escrow |
| `r402-tvm` | 23 | 6 800 | TON exact (W5R1 / Highload V3) |
| `r402-http` | 10 | 5 381 | Axum paygate + reqwest client + remote facilitator HTTP |
| `r402-stellar` | 17 | 5 170 | Stellar exact |
| `r402-casper` | 16 | 4 748 | Casper exact (local preflight + remote facilitator) |
| `r402-algorand` | 17 | 4 532 | Algorand exact |
| `r402-xrpl` | 16 | 4 491 | XRPL exact |
| `r402-hedera` | 16 | 4 222 | Hedera exact |
| `r402-near` | 15 | 3 857 | NEAR exact |
| `r402-aptos` | 15 | 3 660 | Aptos exact |
| `r402-keeta` | 15 | 3 473 | Keeta exact |
| `r402-tron` | 15 | 3 291 | Tron exact (r402-only vs official TS) |
| `r402-mcp` | 5 | 2 617 | MCP transport on `rmcp` |
| **sum** | **295** | **86 256** | |

---

## Full table

Path | lines | real responsibility | God/junk? | notes
--- | ---: | --- | --- | ---
`crates/r402/src/lib.rs` | 48 | Feature-gated `pub use` of sibling crates | | No business. Keep as umbrella shape.

### `r402-core` (God crate: protocol + orchestration mixed)

Path | lines | real responsibility | God/junk? | notes
--- | ---: | --- | --- | ---
`crates/r402-core/src/lib.rs` | 85 | Mods + large `pub use` surface | | Leaks ResourceServer internals into crate root.
`crates/r402-core/src/amount.rs` | 245 | Parse `"$0.01"` / `"1,000"` into `MoneyAmount` | | Domain: money. Not junk.
`crates/r402-core/src/cache.rs` | 249 | `moka` TTL set / settlement duplicate cache | | Feature `cache`. Facilitator concern.
`crates/r402-core/src/chain.rs` | 509 | CAIP-2 `ChainId` / `ChainIdPattern` / registry / token amount | | Mixes identifier types with provider traits. Split.
`crates/r402-core/src/client.rs` | 1444 | Official `x402Client`: register, policies, spendControls, sign, `on_payment_response` | **God** | One file, selection + spend + hooks + creation.
`crates/r402-core/src/error.rs` | 641 | `VerificationError` / `SettlementError` / `FacilitatorError` / `ErrorReason` / `PaymentProblem` | | Over 400; split reason catalogue from typed errors.
`crates/r402-core/src/extensions/mod.rs` | 355 | `Extension` / `DynExtension` / `ExtensionRegistry` + contexts | | Framework, not a drawer.
`crates/r402-core/src/extensions/bazaar.rs` | 127 | Bazaar discovery advertise/echo | | Feature `ext-bazaar`.
`crates/r402-core/src/extensions/eip2612.rs` | 98 | EIP-2612 gas-sponsoring advertise | | Payload types live in `r402-evm`.
`crates/r402-core/src/extensions/erc20_approval.rs` | 95 | ERC-20 approval gas-sponsoring advertise | |
`crates/r402-core/src/extensions/payment_id.rs` | 320 | Payment-identifier idempotency | | Depends on `cache`.
`crates/r402-core/src/facilitator.rs` | 768 | `Facilitator` AFIT + `DynFacilitator` + `HookedFacilitator` + hook enums | | Over 400; split trait / dyn / hooks.
`crates/r402-core/src/metrics.rs` | 96 | Prometheus-style metric name catalogue (`r402_*`) | |
`crates/r402-core/src/pending_settlement.rs` | 200 | `PendingSettlementStore` for `settlement_pending` | |
`crates/r402-core/src/resource_server/mod.rs` | 2505 | `ResourceServer`: 402 build, match, verify, settle phases, cancel | **God** | Largest production file. Orchestrator + helpers.
`crates/r402-core/src/resource_server/hook_policy.rs` | 869 | Official hook mutation policy (additive extra, settle core snapshot) | **God** | Policy vs snapshots mixed.
`crates/r402-core/src/resource_server/hooks.rs` | 461 | `ResourceServerHooks`, `CancelReason`, verify/settle decisions | | Over 400.
`crates/r402-core/src/resource_server/payment_flow.rs` | 779 | Closed `authorization` / `upfront` / `escrow` + `SettlePhase` | | 700–800 应拆，不必叫 God。
`crates/r402-core/src/resource_server/scheme.rs` | 199 | `SchemeNetworkServer` adapter trait | |
`crates/r402-core/src/scheme/mod.rs` | 198 | `ExactScheme` / `UptoScheme` / `BatchSettlementScheme` / `AuthCaptureScheme` markers + `Sealed` | | `Sealed` blocks out-of-tree plugins.
`crates/r402-core/src/scheme/client.rs` | 227 | `SchemeClient`, `PaymentCandidate`, selector/policy traits | |
`crates/r402-core/src/scheme/registry.rs` | 361 | `SchemeRegistry` / `SchemeBuilder` / `SchemeBlueprint` | | Extra trait layer on top of `Facilitator`.
`crates/r402-core/src/wire/mod.rs` | 33 | Re-exports | |
`crates/r402-core/src/wire/codec.rs` | 271 | `Version<2>`, `Base64Bytes`, `U64String`, `UnixTimestamp` | |
`crates/r402-core/src/wire/extensions.rs` | 219 | `Extensions` map / `ExtensionEntry` | |
`crates/r402-core/src/wire/offer.rs` | 704 | `ResourceInfo`, `PaymentRequirements`, `PaymentRequired`, `PaymentPayload`, `PriceTag` | | 700–800 应拆，不必叫 God。
`crates/r402-core/src/wire/rpc.rs` | 1240 | Verify/settle/`/supported` envelopes, `SettlementOverrides` | **God** | Opaque `VerifyRequest(Value)` + typed overlay + overrides.
`crates/r402-core/tests/public_api.rs` | 207 | Public-API characterization (wire roundtrip, deny_unknown_fields) | | Error string `"missing X-PAYMENT"` is V1 leftover.
`crates/r402-core/tests/spec_fixtures.rs` | 170 | Load `tests/fixtures/spec_v2/*.json` | |
`crates/r402-core/tests/wire_proptest.rs` | 367 | Property tests for wire codecs | |

### `r402-http`

Path | lines | real responsibility | God/junk? | notes
--- | ---: | --- | --- | ---
`crates/r402-http/src/lib.rs` | 27 | Feature-gated `client` / `server` mods | |
`crates/r402-http/src/client.rs` | 400 | reqwest-middleware: 402 → `Payment-Signature` retry + recovery | | Boundary unwrap on header insert.
`crates/r402-http/src/server/mod.rs` | 42 | Re-exports including `SettlementMode` | | Settlement modes buried here, not first-class dir.
`crates/r402-http/src/server/paygate.rs` | 2278 | Extract header, verify, three settlement modes, 402/response headers | **God** | Sequential / Concurrent / Background all in one file.
`crates/r402-http/src/server/middleware.rs` | 683 | `X402Middleware` / `X402Layer` Tower/Axum | | Over 400; builder vs service mixed.
`crates/r402-http/src/server/facilitator.rs` | 1288 | Remote `FacilitatorClient` (`/verify` `/settle` `/supported`) | **God** | HTTP facilitator client living in server module.
`crates/r402-http/src/server/hooks.rs` | 97 | HTTP-only `PaygateHooks` (API key / KYT bypass) | |
`crates/r402-http/src/server/pricing.rs` | 133 | Static / dynamic `PriceTagSource` | |
`crates/r402-http/src/server/tracker.rs` | 174 | `BackgroundSettlementTracker` in-flight counter | |
`crates/r402-http/src/server/upto.rs` | 259 | `Settlement-Overrides` header + `UptoActualAmount` | | Sequential-only honor of actual amount.

### `r402-mcp`

Path | lines | real responsibility | God/junk? | notes
--- | ---: | --- | --- | ---
`crates/r402-mcp/src/lib.rs` | 97 | Constants (`x402/payment` meta keys, JSON-RPC 402 / -32042) | |
`crates/r402-mcp/src/encode.rs` | 496 | Dual-format 402 tool result + `_meta` attach/extract | | Over 400.
`crates/r402-mcp/src/error.rs` | 171 | MCP client/server error types | |
`crates/r402-mcp/src/client.rs` | 622 | Paid tool call retry (`X402McpClient`) | | Over 400.
`crates/r402-mcp/src/server.rs` | 1231 | `PaymentWrapper` around `rmcp` tool handler + paymentFlow | **God** | Orchestration + encoding + hooks.

### `r402-evm`

Path | lines | real responsibility | God/junk? | notes
--- | ---: | --- | --- | ---
`crates/r402-evm/src/lib.rs` | 111 | Mods + public scheme types | |
`crates/r402-evm/src/asset.rs` | 105 | `AssetTransferMethod`, validator address | |
`crates/r402-evm/src/error.rs` | 88 | `Eip155ExactError` (shared across schemes) | |
`crates/r402-evm/src/networks.rs` | 887 | CAIP-2 network table + USDC deployments | **God** | Data table + helpers in one file.
`crates/r402-evm/src/permit2.rs` | 152 | Permit2 address + approval calldata | |
`crates/r402-evm/src/eip2612.rs` | 192 | EIP-2612 permit parse/types | |
`crates/r402-evm/src/erc20_approval.rs` | 119 | ERC-20 approval gas-sponsoring info | |
`crates/r402-evm/src/signature.rs` | 127 | EIP-1271 / 6492 / 2098 dispatch | |
`crates/r402-evm/src/signer.rs` | 41 | `SignerLike` bound | |
`crates/r402-evm/src/chain/mod.rs` | 41 | Re-exports | |
`crates/r402-evm/src/chain/types.rs` | 394 | EVM addresses, deployments | |
`crates/r402-evm/src/chain/provider.rs` | 465 | Alloy provider construction | | Over 400.
`crates/r402-evm/src/chain/nonce.rs` | 95 | Pending-aware nonce manager | |
`crates/r402-evm/src/chain/contracts.rs` | 39 | Contract addresses | |
`crates/r402-evm/src/exact/mod.rs` | 59 | `Eip155Exact` scheme id | |
`crates/r402-evm/src/exact/types.rs` | 360 | EIP-3009 payload / extra | |
`crates/r402-evm/src/exact/client.rs` | 569 | Buyer EIP-712 / 3009 / Permit2 signing | | Over 400.
`crates/r402-evm/src/exact/server.rs` | 91 | Price tag + `SchemeNetworkServer` | |
`crates/r402-evm/src/exact/facilitator/mod.rs` | 358 | Exact facilitator wiring | |
`crates/r402-evm/src/exact/facilitator/verify.rs` | 573 | Exact verify (sig, balance, simulate) | | Over 400.
`crates/r402-evm/src/exact/facilitator/settle.rs` | 377 | Exact on-chain settle | |
`crates/r402-evm/src/exact/facilitator/contract.rs` | 90 | ERC-3009 contract calls | |
`crates/r402-evm/src/exact/facilitator/signature.rs` | 50 | Signature format helpers | |
`crates/r402-evm/src/upto/mod.rs` | 97 | `Eip155Upto` scheme id | |
`crates/r402-evm/src/upto/types.rs` | 310 | Permit2 witness payload | |
`crates/r402-evm/src/upto/server.rs` | 333 | Upto server / ATM / paymentFlow | |
`crates/r402-evm/src/upto/client/mod.rs` | 281 | `Eip155UptoClient` | |
`crates/r402-evm/src/upto/client/signing.rs` | 171 | Permit2 signing | |
`crates/r402-evm/src/upto/client/eip2612.rs` | 186 | Client-side EIP-2612 permit | |
`crates/r402-evm/src/upto/facilitator/mod.rs` | 247 | Upto facilitator wiring | |
`crates/r402-evm/src/upto/facilitator/verify.rs` | 1017 | Upto verify (witness, amount ≤ max) | **God** |
`crates/r402-evm/src/upto/facilitator/settle.rs` | 154 | Upto settle via proxy | |
`crates/r402-evm/src/upto/facilitator/contract.rs` | 109 | Upto proxy contract | |
`crates/r402-evm/src/auth_capture/mod.rs` | 293 | `Eip155AuthCapture` scheme | |
`crates/r402-evm/src/auth_capture/types.rs` | 260 | Auth/capture payload | |
`crates/r402-evm/src/auth_capture/client.rs` | 351 | Buyer auth-capture signing | |
`crates/r402-evm/src/auth_capture/server.rs` | 108 | Server adapter | |
`crates/r402-evm/src/auth_capture/facilitator.rs` | 239 | Facilitator | |
`crates/r402-evm/src/auth_capture/verify.rs` | 266 | Verify | |
`crates/r402-evm/src/auth_capture/nonce.rs` | 108 | Auth-capture nonce | |
`crates/r402-evm/src/batch_settlement/mod.rs` | 147 | `Eip155BatchSettlement` | |
`crates/r402-evm/src/batch_settlement/types.rs` | 286 | Channel / voucher types | |
`crates/r402-evm/src/batch_settlement/client.rs` | 212 | Voucher signing | |
`crates/r402-evm/src/batch_settlement/server.rs` | 89 | Server adapter | |
`crates/r402-evm/src/batch_settlement/facilitator.rs` | 169 | Facilitator | |
`crates/r402-evm/src/batch_settlement/verify.rs` | 103 | Verify voucher | |
`crates/r402-evm/src/batch_settlement/channel.rs` | 129 | Channel id / state | |
`crates/r402-evm/src/batch_settlement/store.rs` | 126 | `ChannelStore` / memory | |
`crates/r402-evm/src/batch_settlement/voucher.rs` | 151 | Voucher codec | |
`crates/r402-evm/tests/onchain_deployment.rs` | 226 | Optional live RPC (`R402_RPC_URLS`) | |

### `r402-solana`

Path | lines | real responsibility | God/junk? | notes
--- | ---: | --- | --- | ---
`crates/r402-solana/src/lib.rs` | 68 | Mods + `SolanaExact` / `SolanaUpto` | | `crates/README.md` still says exact-only — stale.
`crates/r402-solana/src/networks.rs` | 599 | CAIP-2 + USDC mints | | Over 400.
`crates/r402-solana/src/chain/mod.rs` | 32 | Re-exports | |
`crates/r402-solana/src/chain/types.rs` | 406 | Pubkeys, ATAs, compute budget | | Over 400.
`crates/r402-solana/src/chain/provider.rs` | 480 | RPC client / confirmation | | Over 400.
`crates/r402-solana/src/chain/rpc.rs` | 92 | RPC helpers | |
`crates/r402-solana/src/exact/mod.rs` | 54 | `SolanaExact` | |
`crates/r402-solana/src/exact/types.rs` | 320 | Pre-signed tx payload | |
`crates/r402-solana/src/exact/error.rs` | 117 | Exact errors | |
`crates/r402-solana/src/exact/client.rs` | 439 | Build + sign TransferChecked tx | | Over 400.
`crates/r402-solana/src/exact/server.rs` | 133 | Price tag / ATM | |
`crates/r402-solana/src/exact/facilitator/mod.rs` | 212 | Facilitator wiring (settle inlined — no `settle.rs`) | |
`crates/r402-solana/src/exact/facilitator/verify.rs` | 572 | Instruction layout / fee-payer rules | | Over 400.
`crates/r402-solana/src/exact/facilitator/config.rs` | 112 | Compute-unit caps | |
`crates/r402-solana/src/exact/facilitator/memo.rs` | 113 | Memo instruction checks | |
`crates/r402-solana/src/exact/facilitator/smart_wallet.rs` | 298 | SVM smart-wallet path | |
`crates/r402-solana/src/upto/mod.rs` | 61 | `SolanaUpto` escrow scheme | |
`crates/r402-solana/src/upto/types.rs` | 112 | Upto SVM payload | |
`crates/r402-solana/src/upto/error.rs` | 73 | Error codes | |
`crates/r402-solana/src/upto/client.rs` | 262 | Open-channel client | |
`crates/r402-solana/src/upto/server.rs` | 679 | Escrow `SchemeNetworkServer` + two settles | | Over 400.
`crates/r402-solana/src/upto/shared.rs` | 191 | Leftover helpers used by several upto files | **Junk** | Split into named modules.
`crates/r402-solana/src/upto/facilitator/mod.rs` | 298 | Upto facilitator | |
`crates/r402-solana/src/upto/facilitator/verify.rs` | 298 | Verify deposit / voucher | |
`crates/r402-solana/src/upto/facilitator/settle.rs` | 607 | `settle_and_seal` + distribute | | Over 400.
`crates/r402-solana/src/upto/facilitator/config.rs` | 77 | Escrow config | |
`crates/r402-solana/src/upto/facilitator/storage.rs` | 77 | Channel storage | |
`crates/r402-solana/src/upto/payment_channels/mod.rs` | 49 | Re-exports | |
`crates/r402-solana/src/upto/payment_channels/channel.rs` | 174 | Channel accounts | |
`crates/r402-solana/src/upto/payment_channels/codec.rs` | 201 | Account codec | |
`crates/r402-solana/src/upto/payment_channels/instructions.rs` | 225 | Program ix | |
`crates/r402-solana/src/upto/payment_channels/open.rs` | 713 | Open/deposit ix | | 700–800 应拆，不必叫 God。
`crates/r402-solana/src/upto/payment_channels/program.rs` | 136 | Program id | |
`crates/r402-solana/src/upto/payment_channels/voucher.rs` | 93 | Voucher | |

### Exact-only chain crates (isomorphic skeleton, asymmetric leaves)

Each of algorand / aptos / casper / hedera / keeta / near / stellar / tron / tvm / xrpl implements `exact` with `chain/` + `exact/{client,server,facilitator,types}` and `networks.rs`. Deviations called out.

Path | lines | real responsibility | God/junk? | notes
--- | ---: | --- | --- | ---
`crates/r402-algorand/src/lib.rs` | 73 | Crate root | |
`crates/r402-algorand/src/networks.rs` | 148 | CAIP-2 + USDC ASA ids | |
`crates/r402-algorand/src/chain/mod.rs` | 39 | Re-exports | |
`crates/r402-algorand/src/chain/types.rs` | 457 | Addresses, ASA, group | | Over 400.
`crates/r402-algorand/src/chain/codec.rs` | 814 | msgpack / tx group encoding | **God** |
`crates/r402-algorand/src/chain/provider.rs` | 124 | algod wrapper | |
`crates/r402-algorand/src/chain/rpc.rs` | 581 | algod REST | | Over 400.
`crates/r402-algorand/src/chain/signer.rs` | 95 | Local signer | |
`crates/r402-algorand/src/exact/mod.rs` | 40 | `AlgorandExact` | |
`crates/r402-algorand/src/exact/types.rs` | 95 | Payload | |
`crates/r402-algorand/src/exact/error.rs` | 57 | Errors | |
`crates/r402-algorand/src/exact/client.rs` | 236 | Atomic group + ASA xfer | |
`crates/r402-algorand/src/exact/server.rs` | 145 | Price tag | |
`crates/r402-algorand/src/exact/facilitator/mod.rs` | 196 | Wiring | |
`crates/r402-algorand/src/exact/facilitator/verify.rs` | 376 | Verify | |
`crates/r402-algorand/src/exact/facilitator/settle.rs` | 161 | Settle | |
`crates/r402-algorand/src/exact/facilitator/tests.rs` | 895 | In-src facilitator tests | **God (tests)** | Move to `tests/`.
`crates/r402-aptos/src/lib.rs` | 86 | Crate root | |
`crates/r402-aptos/src/networks.rs` | 151 | `aptos:1` / `aptos:2` | |
`crates/r402-aptos/src/chain/mod.rs` | 22 | Re-exports | |
`crates/r402-aptos/src/chain/types.rs` | 417 | Addresses / FA | | Over 400.
`crates/r402-aptos/src/chain/codec.rs` | 565 | BCS / tx codec | | Over 400.
`crates/r402-aptos/src/chain/provider.rs` | 330 | aptos-sdk provider | |
`crates/r402-aptos/src/exact/mod.rs` | 39 | `AptosExact` | |
`crates/r402-aptos/src/exact/types.rs` | 85 | Payload | |
`crates/r402-aptos/src/exact/error.rs` | 53 | Errors | |
`crates/r402-aptos/src/exact/client.rs` | 388 | Sponsored FA transfer | |
`crates/r402-aptos/src/exact/server.rs` | 144 | Price tag | |
`crates/r402-aptos/src/exact/facilitator/mod.rs` | 243 | Wiring | |
`crates/r402-aptos/src/exact/facilitator/verify.rs` | 331 | Verify | |
`crates/r402-aptos/src/exact/facilitator/settle.rs` | 105 | Settle | |
`crates/r402-aptos/src/exact/facilitator/tests.rs` | 701 | In-src tests | | Move to `tests/`.
`crates/r402-casper/src/lib.rs` | 95 | Crate root | |
`crates/r402-casper/src/networks.rs` | 199 | `casper:casper` / `casper-test` | |
`crates/r402-casper/src/hex.rs` | 141 | Unprefixed hex codec | **Junk** | Reimplements `hex`; belongs under `chain/codec.rs`.
`crates/r402-casper/src/motes.rs` | 431 | Exact 9-decimal mote arithmetic | | Domain type at crate root. Move to `chain/motes.rs`.
`crates/r402-casper/src/chain/mod.rs` | 26 | Re-exports | |
`crates/r402-casper/src/chain/types.rs` | 854 | Keys, hashes, deployments | **God** |
`crates/r402-casper/src/exact/mod.rs` | 88 | `CasperExact` | |
`crates/r402-casper/src/exact/types.rs` | 386 | CEP-18 payload | |
`crates/r402-casper/src/exact/error.rs` | 217 | Errors | |
`crates/r402-casper/src/exact/eip712.rs` | 221 | TransferWithAuthorization digest | |
`crates/r402-casper/src/exact/client.rs` | 491 | Auth assembly | | Over 400.
`crates/r402-casper/src/exact/server.rs` | 247 | Price tag | |
`crates/r402-casper/src/exact/verify.rs` | 500 | Local preflight verify | | Over 400. Sibling of facilitator, not under it.
`crates/r402-casper/src/exact/facilitator/mod.rs` | 462 | Remote facilitator client | | Over 400.
`crates/r402-casper/src/exact/facilitator/config.rs` | 281 | `R402_CASPER_FACILITATOR_URL` | | Only production env var.
`crates/r402-casper/src/exact/facilitator/transport.rs` | 109 | HTTP transport | |
`crates/r402-hedera/src/lib.rs` | 59 | Crate root | |
`crates/r402-hedera/src/networks.rs` | 206 | `hedera:mainnet` / testnet | |
`crates/r402-hedera/src/chain/mod.rs` | 28 | Re-exports | |
`crates/r402-hedera/src/chain/types.rs` | 488 | Account / token ids | | Over 400.
`crates/r402-hedera/src/chain/provider.rs` | 257 | Hiero SDK | |
`crates/r402-hedera/src/chain/rpc.rs` | 703 | Mirror REST | | Over 400.
`crates/r402-hedera/src/chain/tx.rs` | 357 | TransferTransaction | |
`crates/r402-hedera/src/exact/mod.rs` | 39 | `HederaExact` | |
`crates/r402-hedera/src/exact/types.rs` | 77 | Payload | |
`crates/r402-hedera/src/exact/error.rs` | 57 | Errors | |
`crates/r402-hedera/src/exact/client.rs` | 272 | Signed transfer | |
`crates/r402-hedera/src/exact/server.rs` | 141 | Price tag | |
`crates/r402-hedera/src/exact/facilitator/mod.rs` | 261 | Wiring | |
`crates/r402-hedera/src/exact/facilitator/verify.rs` | 299 | Verify | |
`crates/r402-hedera/src/exact/facilitator/settle.rs` | 113 | Settle | |
`crates/r402-hedera/src/exact/facilitator/tests.rs` | 865 | In-src tests | **God (tests)** |
`crates/r402-keeta/src/lib.rs` | 48 | Crate root | |
`crates/r402-keeta/src/networks.rs` | 162 | `keeta:21378` / test | |
`crates/r402-keeta/src/chain/mod.rs` | 16 | Re-exports | |
`crates/r402-keeta/src/chain/types.rs` | 423 | Accounts / SEND | | Over 400.
`crates/r402-keeta/src/chain/provider.rs` | 269 | keetanetwork-client | |
`crates/r402-keeta/src/exact/mod.rs` | 39 | `KeetaExact` | |
`crates/r402-keeta/src/exact/types.rs` | 110 | Payload | |
`crates/r402-keeta/src/exact/error.rs` | 44 | Errors | |
`crates/r402-keeta/src/exact/client.rs` | 312 | Signed SEND | |
`crates/r402-keeta/src/exact/server.rs` | 104 | Price tag | |
`crates/r402-keeta/src/exact/facilitator/mod.rs` | 147 | Wiring | |
`crates/r402-keeta/src/exact/facilitator/verify.rs` | 252 | Verify | |
`crates/r402-keeta/src/exact/facilitator/settle.rs` | 115 | Settle | |
`crates/r402-keeta/src/exact/facilitator/queue.rs` | 300 | Transmit queue | |
`crates/r402-keeta/src/exact/facilitator/tests.rs` | 1132 | In-src tests | **God (tests)** |
`crates/r402-near/src/lib.rs` | 79 | Crate root | |
`crates/r402-near/src/networks.rs` | 153 | `near:mainnet` / testnet | |
`crates/r402-near/src/chain/mod.rs` | 22 | Re-exports | |
`crates/r402-near/src/chain/types.rs` | 420 | Accounts / NEP-141 | | Over 400.
`crates/r402-near/src/chain/provider.rs` | 295 | JSON-RPC | |
`crates/r402-near/src/chain/rpc.rs` | 569 | RPC methods | | Over 400.
`crates/r402-near/src/exact/mod.rs` | 39 | `NearExact` | |
`crates/r402-near/src/exact/types.rs` | 71 | Payload | |
`crates/r402-near/src/exact/error.rs` | 44 | Errors | |
`crates/r402-near/src/exact/client.rs` | 273 | NEP-366 delegate | | Test reads `NEAR_ACCOUNT_ID` / `NEAR_SECRET_KEY`.
`crates/r402-near/src/exact/server.rs` | 93 | Price tag | |
`crates/r402-near/src/exact/facilitator/mod.rs` | 167 | Wiring | |
`crates/r402-near/src/exact/facilitator/verify.rs` | 291 | Verify | |
`crates/r402-near/src/exact/facilitator/settle.rs` | 133 | Relayer settle | |
`crates/r402-near/src/exact/facilitator/tests.rs` | 1208 | In-src tests | **God (tests)** |
`crates/r402-stellar/src/lib.rs` | 92 | Crate root | |
`crates/r402-stellar/src/networks.rs` | 167 | `stellar:pubnet` / testnet | |
`crates/r402-stellar/src/chain/mod.rs` | 34 | Re-exports | |
`crates/r402-stellar/src/chain/types.rs` | 540 | Addresses / SEP-41 | | Over 400.
`crates/r402-stellar/src/chain/provider.rs` | 308 | stellar-rpc-client | |
`crates/r402-stellar/src/chain/rpc.rs` | 493 | RPC | | Over 400.
`crates/r402-stellar/src/chain/signer.rs` | 117 | Auth-entry signer | |
`crates/r402-stellar/src/chain/xdr.rs` | 680 | XDR encode | | Over 400.
`crates/r402-stellar/src/exact/mod.rs` | 39 | `StellarExact` | |
`crates/r402-stellar/src/exact/types.rs` | 100 | Payload | |
`crates/r402-stellar/src/exact/error.rs` | 57 | Errors | |
`crates/r402-stellar/src/exact/client.rs` | 435 | Auth-entry signing | | Over 400. Test reads `STELLAR_SECRET_KEY`.
`crates/r402-stellar/src/exact/server.rs` | 162 | Price tag | |
`crates/r402-stellar/src/exact/facilitator/mod.rs` | 190 | Wiring | |
`crates/r402-stellar/src/exact/facilitator/verify.rs` | 366 | Verify | |
`crates/r402-stellar/src/exact/facilitator/settle.rs` | 168 | Settle | |
`crates/r402-stellar/src/exact/facilitator/tests.rs` | 1222 | In-src tests | **God (tests)** |
`crates/r402-tron/src/lib.rs` | 54 | Crate root | | r402-only vs official TS.
`crates/r402-tron/src/networks.rs` | 248 | `tron:0x2b6653dc` / Nile | |
`crates/r402-tron/src/chain/mod.rs` | 31 | Re-exports | |
`crates/r402-tron/src/chain/types.rs` | 511 | Addresses / TIP-712 | | Over 400.
`crates/r402-tron/src/chain/provider.rs` | 522 | TronGrid HTTP | | Over 400.
`crates/r402-tron/src/chain/contracts.rs` | 66 | Contract ids | |
`crates/r402-tron/src/exact/mod.rs` | 56 | `TronExact` | |
`crates/r402-tron/src/exact/types.rs` | 281 | EIP-3009-like payload | |
`crates/r402-tron/src/exact/error.rs` | 97 | Errors | |
`crates/r402-tron/src/exact/client.rs` | 374 | TIP-712 / Permit2 | |
`crates/r402-tron/src/exact/server.rs` | 99 | Price tag | |
`crates/r402-tron/src/exact/facilitator/mod.rs` | 316 | Wiring | |
`crates/r402-tron/src/exact/facilitator/verify.rs` | 471 | Verify | | Over 400.
`crates/r402-tron/src/exact/facilitator/settle.rs` | 83 | Settle | |
`crates/r402-tron/src/exact/facilitator/signature.rs` | 82 | Sig helpers | |
`crates/r402-tvm/src/lib.rs` | 184 | Crate root + many DEFAULT_* constants | | Constants belong in `chain/` or `exact/`.
`crates/r402-tvm/src/networks.rs` | 146 | `tvm:-239` / `tvm:-3` | |
`crates/r402-tvm/src/provider.rs` | 808 | Toncenter / TonAPI REST (crate-root duplicate) | **God + Junk** | Parallel to `chain/provider.rs`.
`crates/r402-tvm/src/trace.rs` | 170 | Debug tracing helpers | **Junk** | Observability leftover at crate root.
`crates/r402-tvm/src/chain/mod.rs` | 30 | Re-exports | |
`crates/r402-tvm/src/chain/types.rs` | 405 | Addresses / jetton | | Over 400.
`crates/r402-tvm/src/chain/provider.rs` | 633 | Chain provider | | Over 400. Overlaps root `provider.rs`.
`crates/r402-tvm/src/chain/rpc.rs` | 198 | RPC | |
`crates/r402-tvm/src/codecs/mod.rs` | 18 | Drawer of BoC codecs | **Junk drawer** | Move under `chain/codec/`.
`crates/r402-tvm/src/codecs/common.rs` | 146 | Shared cell helpers | **Junk drawer** |
`crates/r402-tvm/src/codecs/w5.rs` | 482 | W5R1 wallet codec | **Junk drawer** | Domain: `chain/codec/w5.rs`.
`crates/r402-tvm/src/codecs/highload_v3.rs` | 264 | Highload V3 codec | **Junk drawer** |
`crates/r402-tvm/src/codecs/jetton.rs` | 173 | TEP-74 jetton | **Junk drawer** |
`crates/r402-tvm/src/exact/mod.rs` | 54 | `TvmExact` | |
`crates/r402-tvm/src/exact/types.rs` | 156 | Payload | |
`crates/r402-tvm/src/exact/error.rs` | 127 | Errors | |
`crates/r402-tvm/src/exact/client.rs` | 557 | W5R1 settlement BoC | | Over 400.
`crates/r402-tvm/src/exact/server.rs` | 134 | Price tag | |
`crates/r402-tvm/src/exact/settlement_batcher.rs` | 734 | Highload V3 batcher | | 700–800 应拆，不必叫 God。Facilitator concern.
`crates/r402-tvm/src/exact/facilitator/mod.rs` | 209 | Wiring | |
`crates/r402-tvm/src/exact/facilitator/verify.rs` | 520 | Verify | | Over 400.
`crates/r402-tvm/src/exact/facilitator/settle.rs` | 107 | Settle | |
`crates/r402-tvm/src/exact/facilitator/tests.rs` | 545 | In-src tests | |
`crates/r402-xrpl/src/lib.rs` | 98 | Crate root | |
`crates/r402-xrpl/src/networks.rs` | 207 | `xrpl:0/1/2` | |
`crates/r402-xrpl/src/chain/mod.rs` | 28 | Re-exports | |
`crates/r402-xrpl/src/chain/types.rs` | 543 | Addresses / Payment | | Over 400.
`crates/r402-xrpl/src/chain/codec.rs` | 237 | Blob codec | |
`crates/r402-xrpl/src/chain/provider.rs` | 129 | JSON-RPC | |
`crates/r402-xrpl/src/chain/rpc.rs` | 704 | RPC methods | | Over 400.
`crates/r402-xrpl/src/exact/mod.rs` | 38 | `XrplExact` | |
`crates/r402-xrpl/src/exact/types.rs` | 151 | Payload | |
`crates/r402-xrpl/src/exact/error.rs` | 50 | Errors | |
`crates/r402-xrpl/src/exact/client.rs` | 572 | Payer-signed Payment blob | | Over 400.
`crates/r402-xrpl/src/exact/server.rs` | 148 | Price tag | |
`crates/r402-xrpl/src/exact/facilitator/mod.rs` | 159 | Wiring | |
`crates/r402-xrpl/src/exact/facilitator/verify.rs` | 555 | Verify | | Over 400.
`crates/r402-xrpl/src/exact/facilitator/settle.rs` | 232 | Settle | |
`crates/r402-xrpl/src/exact/facilitator/tests.rs` | 640 | In-src tests | |

---

## Structural problems (inventory, not a plan)

1. **`r402-core` is a God crate.** Wire types, CAIP, client, resource server, facilitator, extensions, metrics, cache share one compile unit and one `lib.rs` re-export dump.
2. **Settlement modes are not a directory.** `SettlementMode` lives inside `paygate.rs` (2278 lines) even though README treats the three modes as a product feature.
3. **HTTP facilitator client is nested under `server/`.** Remote `/verify` is not an Axum concern.
4. **Chain crates are almost isomorphic but not.** Solana exact has no `settle.rs`; Casper verify sits beside facilitator; TVM has two providers and a `codecs/` drawer; Casper has `hex.rs` + `motes.rs` at crate root.
5. **In-src `facilitator/tests.rs` bags** (algorand 895, hedera 865, keeta 1132, near 1208, stellar 1222) hide unit tests inside production modules.
6. **No CLI, no examples.** Happy path exists only in README snippets.
7. **Sealed `SchemeClient`.** Out-of-tree chain plugins cannot implement the trait.
8. **No `sign-in-with-x` extension** despite official spec and in-org `siwx`.
9. **Docs drift:** `crates/README.md` omits Solana `upto`; `r402-http/README.md` lists a `facilitator` Cargo feature that `Cargo.toml` does not define.
10. **Junk names:** `hex`, `shared`, `codecs`, `trace`, crate-root `provider.rs`.
11. Non-`.rs` fixtures are product vectors (FILE_MAP must list them or PR-delete loses them).
12. HTTP matching bypass through `PriceTag == accepted` is **already closed**; paygate uses `ResourceServer::find_matching_requirements`. Keep-current.
