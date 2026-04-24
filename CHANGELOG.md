# Changelog

All notable changes to the `r402` workspace are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.14.0-beta.1] — 2026-04-24

### Breaking changes

- **Crate layout renamed.** The former `r402` core crate is now [`r402-core`]; a new facade crate `r402` re-exports every chain, transport, and extension under feature flags. Downstream consumers should depend on `r402` and enable the features they need.
- **Wire types sealed and strict.** All JSON envelopes carry `#[serde(deny_unknown_fields)]`, identifier / network / scheme / amount fields switched from `String` to `CompactString`, and public enums are `#[non_exhaustive]`. The legacy `proto::` module was renamed to `wire::` with an explicit v2 namespace.
- **`Facilitator` trait migrated to AFIT.** Native `async fn` replaces the previous `BoxFuture`-returning signature. A new `DynFacilitator` dyn-safe shim provides object-safety, with blanket impls over every `T: Facilitator`.
- **`FacilitatorHooks` and `PaygateHooks` are AFIT.** The hook runners now require the inner facilitator be `Sync`. A `DynFacilitatorHooks` shim mirrors `DynFacilitator`.
- **Sealed scheme trait hierarchy.** `SchemeId`, `SchemeServer`, and `SchemeClient` are now sealed; only `ExactScheme` and `UptoScheme` can be registered. Out-of-tree extensions must register via the extension trait framework instead.
- **Typestate `Payment<S>`.** The new `payment` module exposes `Payment<Unverified>` → `verify` → `Payment<Verified>` → `settle` → `Payment<Settled>`, with compile-time enforcement of lifecycle ordering.
- **Layered error enums.** `VerificationError`, `SettlementError`, `FacilitatorError`, and `ClientError` replace the former monolithic `Error`. Every variant carries an `ErrorReason` mapping via `AsPaymentProblem::as_payment_problem`.
- **`PaymentRequirements<TScheme, TAmount, TAddress, TExtra>`.** Made generic; chain crates instantiate concrete variants. The wire-level alias keeps the all-compact-string shape.
- **`SettleResponse::Success.amount`.** Facilitators now report the on-chain amount that actually moved (required by x402 v2 §Settlement Response). The value is always present on success.
- **`ResourceInfo.description` / `mime_type` optional.** Fields now use `Option<CompactString>`, matching the v2 spec where both are optional.
- **MSRV raised to 1.91.** Required for `Duration::from_mins` and const slice methods in the Solana provider / HTTP server modules.

### Added

- **P0 fixes (Fix-1 … Fix-9).**
  - **Fix-1:** `r402-evm` enforces `transfer.value == maxAmountRequired` exactly via the new `assert_exact_value` helper (previously `>=`, violating x402 v2 "exact" semantics).
  - **Fix-2:** `r402-svm` `SolanaExactFacilitator` carries a `SettlementCache` TTL-deduplicator keyed by the base-64 transaction payload. Replays within the (2× blockhash lifetime, 120 s) window fail with `VerificationError::DuplicateSettlement`.
  - **Fix-3:** `r402-svm` honours `SupportedPaymentKindExtra::memo`. Facilitators enforce exactly one SPL Memo instruction whose data bytes equal the declared memo and whitelist the SPL Memo program (v2) + Solflare Lighthouse.
  - **Fix-5:** HTTP paygate maps `ErrorReason::Permit2AllowanceRequired` to HTTP 412 (all other reasons remain 402).
  - **Fix-6:** HTTP paygate sets `Access-Control-Expose-Headers: Payment-Required, Payment-Response` on every `402` / `412` response, merging with any existing value idempotently.
  - **Fix-7:** `r402-evm` `verify_permit2_payment` simulates the `x402Permit2Proxy.settle` call via `eth_call` after signature verification, catching nonce / allowance / deadline reverts before gas is committed.
  - **Fix-8:** HTTP paygate exposes `PaygateHooks` (`on_protected_request` / `on_payment_verified`) with a `DynPaygateHooks` dyn-safe shim. Hooks can grant access, short-circuit with an arbitrary status, or stamp request extensions.
  - **Fix-9:** `r402-svm` `SolanaExactFacilitatorConfig.max_instruction_count` default lowered from 10 to 6 per x402 v2 §SVM exact scheme.
- **Extension framework (`r402-core::extensions`).** `Extension` trait, `DynExtension` shim, `ExtensionRegistry`, `AdvertiseContext`, `VerifyContext`, and `SettleContext` with built-in `BazaarExtension` (resource-discovery metadata) and `PaymentIdentifierExtension` (idempotency key with TTL cache). Both ship behind the `cache` feature.
- **TTL cache primitive.** `r402-core::cache::TtlSet` and the `Duplicate` outcome type.
- **CORS / status helpers.** `r402-http::server::{cors, status}`: `X402_EXPOSED_HEADERS`, `ensure_expose_headers`, and `reason_to_status` are now part of the public API.
- **`r402-mcp` crate.** Placeholder for the forthcoming MCP transport implementation.
- **Upto scheme (`r402-evm::upto`).** Implements the x402 v2 `upto` scheme (`schemes/upto/scheme_upto_evm.md`): Permit2-only `PermitWitnessTransferFrom` with a witness carrying `to`, `validAfter`, and optional `extra` bytes. The buyer signs a maximum, and the resource server settles for any amount in `[0, max]` (`actualAmount == 0` triggers a zero-shortcut with no on-chain tx). Ships with `Eip155Upto` (server price-tag), `Eip155UptoClient` (buyer signer), `Eip155UptoFacilitator` (verify + settle), `UptoPermit2Payload` wire type, dedicated `X402_UPTO_PERMIT2_PROXY` address (`0x4020A4f3b7b90ccA423B9fabCc0CE57C6C240002`), and `ErrorReason::SettlementExceedsAmount` (`invalid_upto_evm_payload_settlement_exceeds_amount`). `SettleRequest::set_settlement_amount` lets resource servers override `paymentRequirements.amount` in-place.
- **HTTP upto bridge (`r402-http::server::upto`).** `UptoActualAmount` response extension lets handlers communicate usage-based charges back to the middleware. `VerifiedPayment::settle_with_override` forwards the amount to the facilitator. Honoured by `SettlementMode::Sequential`; concurrent / background modes settle at the signed maximum.

### Fixed

- `Eip155ExactFacilitator::settle` now populates `SettleResponse.amount` from `paymentRequirements.amount` — previously the field was always `None`.
- `SolanaExactFacilitator::settle` populates `SettleResponse.amount` analogously.
- Removed the unsafe `IndexMut`-style JSON access inside `r402-core`'s production code paths.

### Removed

- `r402-ext` crate (merged into `r402-core::extensions`).
- `BoxFuture`-returning variants of `Facilitator::verify` / `settle` / `supported`; call sites should move to the AFIT signatures or use the `DynFacilitator` shim.
- `as_registered` builder method on `BazaarExtension` — renamed to `registered` to satisfy `clippy::wrong_self_convention`.

[0.14.0-beta.1]: https://github.com/qntx/r402/releases/tag/v0.14.0-beta.1
[`r402-core`]: https://docs.rs/r402-core/0.14.0-beta.1
