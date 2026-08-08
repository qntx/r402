# Changelog

All notable changes to the `r402` workspace are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Protocol alignment (x402 foundation / Go / TS)

- **`r402-svm` exact amount rule** now matches `scheme_exact_svm.md` §1.4
  and the official Go/TS facilitators: on-chain `TransferChecked` amount
  must be **`>=` required** (overpayment allowed for wallet fee-rounding).
  Underpayment still fails with `VerificationError::InvalidPaymentAmount`.
  **Breaking** relative to the previous strict-equality behaviour.
- **`SolanaExactFacilitator::settle`** reports the **actual** on-chain
  transfer amount in `SettleResponse.amount` (not the requirements floor).
- **`AssetTransferMethod`** rejects unknown wire values (including
  `erc7710`) with an explicit unsupported-method error. ERC-7710 settle
  is not implemented (spec-only; not present in foundation Go/TS packages).
- **`r402-http` Paygate** exposes `Payment-Response` via
  `Access-Control-Expose-Headers` on sequential and concurrent **success**
  paths (error paths already did).

### Documentation honesty

- Removed dead `../audit/` links from crate READMEs (no audit tree in-repo).
- Clarified EVM network table as default stablecoin deployments (not all USDC).
- Marked `r402-mcp` as a non-production placeholder in the crate catalog.

Comprehensive audit pass against `3rdparty/x402/specs/`. **31 P1/P2
findings** addressed plus four new regression-test suites. The release
is fully wire- and behaviour-compatible with the v2 spec; the breaking
changes below are API ergonomics (sealed wire types, builder APIs)
rather than protocol semantics.

### Breaking changes

- **`#[non_exhaustive]` on every public wire envelope** (F-115).
  `PaymentRequired`, `PaymentRequirements<…>`, `PaymentPayload<…>`,
  `ResourceInfo`, `SupportedPaymentKind`, `SupportedResponse`,
  `Extensions::ExtensionEntry`, plus the hook / extension contexts
  (`HookVerifyContext`, `HookSettleContext`, `AdvertiseContext`,
  `VerifyContext`, `SettleContext`) are sealed against direct struct
  literals. Construct via the new builder APIs (`new(...)`, fluent
  `with_*` setters) so server / facilitator code stays source-compatible
  when the spec gains new optional fields.
- **`SettleResponse::Failure.transaction` always serialises** (F-002).
  The wire format now emits `"transaction": ""` on every failure
  response, matching spec §5.3.2 and the Go / TS reference SDKs.
  Consumers that previously treated absence of `transaction` as a
  failure marker must now check `success: false` instead.
- **`SettlementCache` moved to `r402-core::cache`** (F-101). The
  former SVM-specific cache is replaced by a chain-agnostic
  `SettlementCache<TKey>` reused by EVM `exact` / `upto` facilitators
  and the SVM `exact` facilitator. The SVM type alias is preserved
  during the deprecation window via a re-export.

### Added

- **`r402-casper` chain crate** — Casper Network support for the x402
  `exact` scheme (layout aligned with other chain crates; **settlement
  is remote** via a Casper facilitator, not an in-process RPC stack):
  - Networks `casper:casper` (mainnet) and `casper:casper-test`
    (testnet) with CAIP-2 chain IDs, plus a built-in wCSPR CEP-18
    deployment table and the `WCSPR` accessor.
  - Local preflight + facilitator client (`verify` / `settle` /
    `supported`) targeting `https://x402-facilitator.cspr.cloud`,
    overridable through `R402_CASPER_FACILITATOR_URL`.
  - 9-decimal mote arithmetic is exact integer math — sub-mote
    precision returns a typed `MotesParseError` rather than silently
    truncating.

- **`BackgroundSettlementTracker`** (F-101 / F-102, `r402-http`). RAII
  in-flight counter + `tokio::sync::Notify` for graceful shutdown of
  background settlement tasks.
  - `Paygate::settlement_tracker()` exposes the tracker; operators call
    `BackgroundSettlementTracker::wait_for_drain(timeout)` from their
    shutdown handler to await pending settlements with a deadline.
  - `PaygateBuilder::with_settlement_tracker` wires it in.
  - Lock-free steady state — single `AtomicUsize` + cloneable `Arc`
    inner for sharing across multiple paygates.
- **Metrics facade** (F-100, `r402-core::metrics`, opt-in `metrics`
  feature). Stable Prometheus-/StatsD-/OpenTelemetry-compatible metric
  names: `r402_facilitator_verify_total`,
  `r402_facilitator_settle_total`,
  `r402_facilitator_verify_duration_seconds`,
  `r402_facilitator_settle_duration_seconds`,
  `r402_settlement_cache_reserve_total`,
  `r402_paygate_request_total`,
  `r402_paygate_background_settle_total`. Cardinality is strictly
  controlled — labels never carry addresses, signatures, or payment
  IDs. Backend choice is the operator's; macros expand to no-ops when
  no recorder is installed.
- **Hot-path instrumentation**.
  `SettlementCache::reserve` emits `r402_settlement_cache_reserve_total`
  with `outcome=inserted|duplicate`; the background settlement
  supervisor emits `r402_paygate_background_settle_total` with
  `result=ok|error|panic|cancelled`.
- **Supervised background settlement** (F-103, `r402-http`). Background
  spawn is wrapped in a supervisor that catches panics, downgrades
  `JoinError::is_cancelled()` to a `tracing::warn!`, and logs full
  context (resource, payer, scheme, network) on failure. No more silent
  task drops on shutdown races.
- **cargo-hack feature-matrix CI** (F-117,
  `.github/workflows/feature-matrix.yml`). Runs
  `cargo hack check --workspace --feature-powerset --depth 2` on every
  push / PR plus a Monday 09:00 UTC schedule, catching upstream feature
  drift.
- **Property-based wire round-trip suite** (F-123,
  `r402-core/tests/wire_proptest.rs`). 10 proptest groups @ 256 cases
  each (≈ 2 560 generated inputs) cover `ResourceInfo`,
  `SupportedPaymentKind`, `SupportedResponse`, `Extensions`,
  `PaymentRequirements`, `PaymentRequired`, `VerifyResponse`,
  `SettleResponse`, and `ErrorReason` (including the `Custom(..)`
  catch-all and totality of `from_wire`).
- **Public-API integration tests** (F-122,
  `r402-core/tests/public_api.rs`, 5 tests). Compiled as a separate
  binary, links r402-core through `pub` only, locks down the F-001 /
  F-002 / wildcard signer behaviours.
- **Spec-fixture cross-SDK suite** (F-121,
  `r402-core/tests/fixtures/spec_v2/` + `tests/spec_fixtures.rs`,
  8 tests). Seven verbatim JSON examples lifted from
  `3rdparty/x402/specs/x402-specification-v2.md` round-trip through
  every wire type. A directory-completeness sanity test fails the build
  if a fixture is added without a matching deserialisation test.

### Fixed

- **`r402-casper` local preflight hardening** (post-merge):
  - `paymentPayload.accepted` must match `paymentRequirements` on
    scheme / network / amount / asset / payTo (EVM/SVM parity).
  - `publicKey` is bound to `authorization.from` via Casper account-hash
    derivation (`blake2b-256(name || 0x00 || key)`); signature algorithm
    tag must match the public-key tag.
  - Validity window uses strict `validAfter < now < validBefore` and
    requires remaining time ≥ `maxTimeoutSeconds` (floor
    `MIN_SETTLEMENT_WINDOW_SECS`).
  - `CasperFacilitatorConfig.timeout` is forwarded to
    `FacilitatorTransport` on every request.
  - Optional `http-client` feature provides `ReqwestTransport` plus
    `CasperExactFacilitator::hosted_http` / `from_env_http`.
  - `CasperExactClient` implements `SchemeClient` for `X402Client`
    registration; CEP-3009 EIP-712 digests follow casper-eip-712 domain
    fields (`name`, `version`, `chain_name`, `contract_package_hash`).
  - `Motes::FromStr` parses base-unit integers (wire form), matching
    `Deserialize` / EVM `TokenAmount` — use `from_cspr_str` for decimals.

- **Docs / facade: surface all chains.** Root README and umbrella
  `r402` document EVM, SVM, Tron, and Casper (crate table, support
  matrix, feature flags). `r402-core` / `r402-mcp` listed. Umbrella
  feature `tron` + crate READMEs for `r402-tron` and `r402-casper`.

- **Scheme markers decode from owned JSON** (`r402-core`).
  `ExactScheme` / `UptoScheme` implemented `Deserialize` via
  `<&str>::deserialize`, which `serde_json::from_value` cannot satisfy.
  This broke `TypedVerifyRequest::from_verify` for every chain crate
  with `invalid type: string "exact", expected a borrowed string`.
  The markers now decode through `Cow<'de, str>`, so both borrowed and
  owned inputs work; covered by a `markers_decode_from_owned_value`
  regression test.
- **F-001** wire envelopes reject typo'd top-level fields
  (`deny_unknown_fields`).
- **F-002** `SettleResponse::Failure` always serialises
  `transaction: ""`.
- **F-003** scheme / network / amount fields use `CompactString`
  instead of `String` (no extra heap allocation for the inline
  case).
- **F-004** `ErrorReason::from_wire` is total — any input maps to
  either a canonical variant or `Custom(...)`.
- **F-016 / F-019 / F-025 / F-028 / F-030 / F-037** — scheme-level
  fixes across `r402-evm` and `r402-svm`.
- **F-044 / F-046 / F-047** Facilitator trait API hardening (`AFIT`,
  `DynFacilitator`, `Sync` requirement on hooks).
- **F-055** HTTP transport headers + status-code mapping aligned with
  spec §7.
- **F-073 / F-074 / F-075 / F-076 / F-077** security audit findings
  (zeroize delegation, PII tracing audit, signature-handling
  invariants, replay-window enforcement).
- **F-143 / F-144 / F-145** maintainability — module reorganisation,
  doctest correctness, public-API documentation completeness.
- **F-117** every clippy lint level promoted to `-D warnings` in
  CI; zero warnings across the whole workspace and every feature
  combination.

### Documentation

- All public APIs documented (`#![warn(missing_docs)]` enforced
  workspace-wide; `cargo doc --all-features --no-deps` reports zero
  warnings).
- crates.io metadata (`keywords`, `categories`, `homepage`) populated
  on every member crate.

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
- **Upto scheme (`r402-evm::upto`).** Implements the x402 v2 `upto` scheme (`schemes/upto/scheme_upto_evm.md`) with full byte-level interop against the official Solidity / TypeScript / Go reference implementations:
  - `Witness(address to, address facilitator, uint256 validAfter)` — the on-chain proxy enforces `msg.sender == witness.facilitator`, so each authorisation is bound to a specific facilitator address.
  - `UptoPaymentRequirementsExtra` carries `name`, `version`, `assetTransferMethod` and the required `facilitatorAddress`. `Eip155Upto::price_tag` ships an enricher that lifts the address from the facilitator's `/supported.signers` map; `Eip155Upto::price_tag_with_facilitator` pins one explicitly.
  - `Eip155UptoFacilitator::supported()` advertises one signer as `kinds[0].extra.facilitatorAddress` and exposes the full set under `signers["eip155:*"]`.
  - `Eip155UptoClient` reads `extra.facilitatorAddress` and embeds it into `witness.facilitator`; the verify / settle pipeline rejects payloads whose witness disagrees with the announced address (`AcceptedRequirementsMismatch`) or is not in the facilitator's signer set (`UptoFacilitatorMismatch`).
  - The facilitator pins the on-chain settle submitter to `witness.facilitator` via the new `MetaTransaction::from` field; sending from any other signer triggers the contract's `UnauthorizedFacilitator` revert.
  - Buyer signs a maximum and the resource server settles for any amount in `[0, max]` (`actualAmount == 0` triggers a zero-shortcut with no on-chain tx). Ships `Eip155Upto`, `Eip155UptoClient`, `Eip155UptoFacilitator`, `UptoPermit2Payload`, the canonical proxy address `X402_UPTO_PERMIT2_PROXY` (`0x4020A4f3b7b90ccA423B9fabCc0CE57C6C240002`), and the `IX402UptoPermit2Proxy` ABI binding (including `settleWithPermit` for the future EIP-2612 extension path and the four canonical revert selectors).
- **Upto error reasons.** `ErrorReason::SettlementExceedsAmount` (`invalid_upto_evm_payload_settlement_exceeds_amount`), `UptoFacilitatorMismatch` (`upto_facilitator_mismatch`), `UptoUnauthorizedFacilitator` (`upto_unauthorized_facilitator`), and `UptoAmountExceedsPermitted` (`upto_amount_exceeds_permitted`) — each backed by a dedicated `VerificationError` variant with structured context. The `eth_call` simulation in verify decodes proxy revert selectors (`AmountExceedsPermitted`, `UnauthorizedFacilitator`, `PaymentTooEarly`, …) into these wire-level reasons; operator-side invariants fall back to `SimulationFailed` with the raw transport message preserved.
- **EIP-2612 gas-sponsoring extension (`r402-evm::upto::eip2612`).** Implements the `eip2612GasSponsoring` payload extension from x402 v2 §upto:
  - `Eip2612SignedPermit` wire type with `from_extensions` / `to_extension_entry` round-trip helpers.
  - Off-chain consistency invariants (`from`, `asset`, `spender`, `value`) cross-validated against the embedded Permit2 authorisation in `assert_offchain_valid_shared`; mismatches surface as `VerificationError::InvalidFormat` before any RPC.
  - `assert_onchain_valid` and `settle_permit2_upto` route through `proxy.settleWithPermit` when sponsorship is attached, atomically broadcasting the EIP-2612 `permit` against the token before invoking the standard Permit2 settle path. The Permit2 allowance precondition is suppressed in this mode (the allowance is granted in-line).
  - Limited to EOA Permit2 signatures by design — smart-contract wallets cannot produce EIP-2612 signatures since the token contract's `ecrecover` rejects ERC-1271 / ERC-6492 envelopes. Mismatched combinations are rejected with `InvalidFormat`.
  - Client-side `sign_eip2612_permit` helper (`r402-evm::upto::client::eip2612`) signs the standard EIP-2612 typed data with a buyer wallet; the token's EIP-712 `name` / `version` are read from `paymentRequirements.extra` and the `nonces(owner)` query is the caller's responsibility.
- **HTTP upto bridge (`r402-http::server::upto`).** `UptoActualAmount` response extension lets handlers communicate usage-based charges back to the middleware. `VerifiedPayment::settle_with_override` forwards the amount to the facilitator. Honoured by `SettlementMode::Sequential`; concurrent / background modes settle at the signed maximum.
- **`MetaTransaction::from`.** New optional pinned-signer field on the EVM meta-transaction primitive. Required by the upto scheme; the exact scheme passes `None` to keep the existing round-robin behaviour.

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
