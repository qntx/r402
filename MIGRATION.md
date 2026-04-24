# Migration guide: 0.13 → 0.14.0-beta.1

The 0.14 release is an enterprise-scale cleanup of the `r402` workspace. This guide walks through every breaking change and how to update downstream code. Assume `r402 = "0.13"` as the starting point.

## 1. Cargo dependency rewrite

The former `r402` crate has been split in two:

- **`r402-core`** — protocol types, traits, extension framework. Depend on this only if you are writing a new chain or transport crate.
- **`r402`** — facade crate that re-exports `r402-core`, the chain crates (`r402-evm`, `r402-svm`), the HTTP transport (`r402-http`), and the upcoming `r402-mcp` behind feature flags.

Update `Cargo.toml`:

```diff
 [dependencies]
-r402 = { version = "0.13", features = ["evm", "http", "server"] }
+r402 = { version = "0.14.0-beta.1", features = ["evm", "http-server"] }
```

The `http` + `server` flags collapsed into `http-server`; `http-client` behaves analogously for buyer-side code.

## 2. `proto::` → `wire::`

Every item previously under `r402::proto` now lives under `r402::wire`.

```diff
-use r402::proto::{PaymentPayload, PaymentRequired, VerifyResponse};
+use r402::wire::{PaymentPayload, PaymentRequired, VerifyResponse};
```

The envelope types additionally:

- use `CompactString` for all identifier / network / scheme / amount fields;
- reject unknown JSON keys (`#[serde(deny_unknown_fields)]`);
- are `#[non_exhaustive]`, so always construct via builders or struct update syntax.

String literals are accepted through `.into()`:

```diff
-let scheme = "exact".to_owned();
+let scheme: CompactString = "exact".into();
```

## 3. `Facilitator` is now AFIT

```diff
 impl Facilitator for MyFacilitator {
-    fn verify(&self, r: VerifyRequest) -> BoxFuture<'_, Result<VerifyResponse, FacilitatorError>> {
-        Box::pin(async move { /* ... */ })
+    async fn verify(&self, r: VerifyRequest) -> Result<VerifyResponse, FacilitatorError> {
+        /* ... */
     }
 }
```

If you need to store a facilitator as a trait object use `Arc<dyn DynFacilitator>` instead of `Arc<dyn Facilitator>`. `DynFacilitator` is blanket-implemented for every `T: Facilitator`, so no manual adapter is required.

## 4. `FacilitatorHooks` + `PaygateHooks`

Hooks moved to AFIT as well:

```diff
-fn before_verify<'a>(&'a self, ctx: &'a VerifyContext) -> BoxFuture<'a, HookDecision> {
-    Box::pin(async { HookDecision::Continue })
-}
+async fn before_verify(&self, _ctx: &VerifyContext) -> HookDecision {
+    HookDecision::Continue
+}
```

The `HookedFacilitator<F>` helper now requires `F: Sync` (required by AFIT to prove `Send` for the generated futures).

The HTTP layer introduces `PaygateHooks` for paygate-layer extensibility (API-key bypass, IP allow-lists, request annotations). Attach them with:

```rust
let layer = X402Middleware::new(facilitator)
    .with_price_tag(price_tag)
    .with_hooks(MyAccessControl::new());
```

## 5. Layered errors

The former `Error` enum split into four crate-rooted variants:

| Old variant | New location |
| --- | --- |
| `Error::Verification(..)` | `VerificationError` |
| `Error::Settlement(..)` | `SettlementError` |
| `Error::Facilitator(..)` | `FacilitatorError` |
| `Error::Client(..)` | `ClientError` |

Every variant implements `AsPaymentProblem::as_payment_problem`, returning a canonical `ErrorReason` and human-readable message.

## 6. Typestate payment lifecycle

The new `Payment<S>` state machine is opt-in. You can still call `facilitator.verify` / `settle` directly, or use:

```rust
use r402::payment::Payment;

let payment = Payment::new(payload);
let (verified, _vr) = payment.verify(&facilitator, req.clone()).await?;
let (_settled, _sr) = verified.settle(&facilitator, req.into()).await?;
```

Calling `settle` on an `Unverified` payment is a compile error.

## 7. `ExactScheme` Permit2 verify simulation (Fix-7)

Verification now runs an additional `eth_call` against `x402Permit2Proxy.settle` to catch revert conditions (expired nonce, revoked allowance, short deadline) before gas is committed. The extra round-trip happens **inside** `verify`; no caller changes required.

If you self-host a facilitator that wraps `verify` make sure the simulation's RPC errors are surfaced to the client rather than suppressed.

## 8. Solana memo + settlement cache

- `SolanaExact::price_tag` continues to produce exact-amount requirements; to require a memo set `SupportedPaymentKindExtra { fee_payer, memo: Some("order-123".into()) }`.
- `SolanaExactFacilitator::new` now attaches a `SettlementCache` with the spec-recommended 120 s TTL. Supply your own via `SolanaExactFacilitator::with_settlement_cache` if you share the deduplication state across workers (Redis adapter is out-of-scope for this release).
- `max_instruction_count` default lowered from 10 to 6.

## 9. CORS + HTTP status

The HTTP paygate emits `Access-Control-Expose-Headers: Payment-Required, Payment-Response` automatically. If you already set that header upstream it is merged, not clobbered.

Verification errors that surface a `permit2_allowance_required` reason now produce `412 Precondition Failed` instead of `402 Payment Required`, per x402 v2.

## 10. MSRV

Raised to **Rust 1.91** (required for `Duration::from_mins` and several `const` slice methods in Solana / HTTP modules). Update your `rust-toolchain.toml` if you pin an older version.

## 11. Upcoming

`UptoScheme` handlers for EVM / SVM are deferred to `0.15.0`. The trait markers (`UptoScheme`, `SchemeId::SLUG`) already exist, but no concrete chain handler is registered. Code that only consumes `ExactScheme` is unaffected.
