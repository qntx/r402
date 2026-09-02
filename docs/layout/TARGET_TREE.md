# TARGET_TREE — r402 2026 corrected

成功 = 禁名不在树 + `r402-contract` JSON 绿 + 无 God 回归 + **下列强制路径都存在**。`.rs` 合计 **384**。`~LOC` 是 review 上限。`lib.rs` 只 mods + `pub use`。

验收 **不是** `wc -l == 384`（RULE 46）。实现 PR 可多 SPLIT God；不可少强制行；禁止把强制孩子并回父文件。

**规范钉。** `/Users/xu/Desktop/x/x402` @ `0344bdf54bf962e758480e76088eb2dc3209a169`（`@x402/*` 2.24.0）。地面真相 HEAD `fb0e1cdfa4990a15cf6a78f7c5ee7e5c646f9b6a` / tag `pre-wipe-0.17.1`。三 SettlementMode KEEP。upto KEEP。HTTP **shape B**。`r402-contract` unpublished。

**施工。** wipe 列出的 crate **目录**（不是 `crates/*` glob）→ 空 `lib.rs` → 自底向上重写。禁止 `git mv`。HEAD 无 `vendor/`；不要 `git rm vendor/`；禁止再加入。

约定：

- 下表 **一行一个将存在的 `.rs` 文件**。
- `Cargo.toml` / `README.md` 每个 crate 各一份，不计入 `.rs` 合计。
- `lib.rs`：只 mods + `pub use`。空壳阶段：`//!` crate 文档 + `crate_loads` 断言本包 `CARGO_PKG_NAME`。
- `foo.rs` + `foo/` 仅当出现第二个文件。不为单孩子建目录。禁止预建 11 个装饰路径。
- 点名的 SPLIT 孩子强制（`algorand/codec/group.rs`）。RULE 9：God 在填入 PR 拆；TARGET 不必预声明未证明的孩子。Casper 强制 17 行。
- 禁名见 RULES / NAMES。无 `X402Route`。无 `into_layer`。无 `settlement/{sequential,concurrent,background,compatibility}.rs`。cors+cache 并入 `headers.rs`。

| Crate | 删这个就去掉 | `.rs` | 相对作废 408 |
| --- | --- | ---: | --- |
| `r402-protocol` | Wire / CAIP / `ErrorReason` | 27 | 同 |
| `r402-client` | Buyer 编排 | 9 | 同 |
| `r402-server` | Resource server + 三 mode + paymentFlow | 19 | 23−4 mode 装饰 |
| `r402-facilitator` | Facilitator trait / cache / HTTP client | 10 | 同 |
| `r402-http` | HTTP 传输 | 27 | 30−`cors.rs`−`cache.rs`−`tests/cache.rs` |
| `r402-mcp` | MCP | 9 | 同 |
| `r402-extensions` | 扩展实现 | 12 | 同 |
| `r402-contract` | JSON 合同断言 | 3 | 同（unpublished） |
| `r402-evm` | EIP-155（含 upto） | 57 | 同 |
| `r402-solana` | Solana（含 upto） | 41 | 同 |
| `r402-algorand` | Algorand exact | 18 | 20−asa−algod；KEEP `codec/group.rs` |
| `r402-aptos` | Aptos exact | 15 | 17−fa−bcs |
| `r402-casper` | Casper exact | 17 | 18−key |
| `r402-hedera` | Hedera exact | 16 | 18−token−mirror |
| `r402-keeta` | Keeta exact | 15 | 16−id |
| `r402-near` | NEAR exact | 15 | 17−nep−query |
| `r402-stellar` | Stellar exact | 17 | 20−muxed−simulate−ops |
| `r402-tron` | Tron exact | 16 | 18−tip−grid |
| `r402-tvm` | TON exact | 24 | 同 |
| `r402-xrpl` | XRPL exact | 16 | 18−classic−ledger |
| `r402` | umbrella | 1 | 同 |
| **合计** | | **384** | 作废 408 |

无 `r402-cli`。无 `r402-core`。21 个 workspace 成员、20 个发布包（`r402-contract` unpublished）。

---

## Workspace root

KEEP：`clippy.toml` `rustfmt.toml` `deny.toml` `workspace.lints` `.github/workflows/*` `LICENSE-*` `AGENTS.md` `CONTRIBUTING.md` `SECURITY.md`。`rust-toolchain.toml`：`channel=stable` + `components=["rustfmt","clippy"]` + `profile=minimal`。`Justfile all: fmt clippy-fix deny test doc-check`。无 Makefile。**无 `vendor/`；禁止加入。**

夹具（点名，已在 WT 则保留）：

```text
tests/fixtures/spec_v2/payment_payload_eip3009.json
tests/fixtures/spec_v2/payment_required.json
tests/fixtures/spec_v2/settle_response_failure.json
tests/fixtures/spec_v2/settle_response_success.json
tests/fixtures/spec_v2/supported_response.json
tests/fixtures/spec_v2/verify_response_invalid.json
tests/fixtures/spec_v2/verify_response_valid.json
tests/fixtures/payment_proptest.proptest-regressions
tests/fixtures/algorand/spec_payment_group.json
tests/fixtures/aptos/extra.json
tests/fixtures/aptos/ts_encode_aptos_payload.json
tests/fixtures/hedera/extra.json
tests/fixtures/keeta/ts_block.b64
tests/fixtures/keeta/ts_payment_payload.json
tests/fixtures/near/ts_signed_delegate.json
tests/fixtures/stellar/extra_sponsored.json
tests/fixtures/stellar/ts_transaction.b64
tests/fixtures/tvm/extra_sponsored.json
tests/fixtures/xrpl/ts_payment_requirements.json
tests/fixtures/contract/headers.json
tests/fixtures/contract/settlement_modes.json
```

Forbidden 根上：`Makefile`、`vendor/`、`tests/contract/*.rs`、`#[ignore]` 锁未存在类型。

### `headers.json` schema

```json
{
  "payment_signature": "Payment-Signature",
  "payment_required": "Payment-Required",
  "payment_response": "Payment-Response",
  "sign_in_with_x": "SIGN-IN-WITH-X",
  "extension_responses": "EXTENSION-RESPONSES",
  "expose_headers": "Payment-Required, Payment-Response",
  "status": {
    "unpaid": 402,
    "malformed_payment": 402,
    "permit2_allowance": 412,
    "facilitator_transport": 502,
    "missing_scheme": 500,
    "incompatible_mode": 500,
    "ok": 200
  },
  "cache_control": {
    "payment_required": "no-store",
    "settle_failure": "no-store",
    "ok_with_receipt": "private"
  }
}
```

### `settlement_modes.json` schema

```json
{
  "sequential": {
    "order": ["verify", "handler", "settle"],
    "payment_response": true,
    "upto_actual": "apply",
    "handler_4xx": "cancel_settle"
  },
  "concurrent": {
    "order": ["verify", "settle_parallel_handler", "join_settle"],
    "payment_response": true,
    "upto_actual": "reject_402_settlement_aborted",
    "handler_4xx": "detach_settle"
  },
  "background": {
    "order": ["verify", "spawn_settle", "handler"],
    "payment_response": false,
    "upto_actual": "strip_do_not_fail",
    "handler_4xx": "detach_settle"
  }
}
```

`r402-contract` 只断言这些字符串/数字。不跑 HTTP。时序的执行锁在 `r402-http/tests/{sequential,concurrent,background}.rs`。

---

## `crates/r402` — 1 `.rs`

| Path | ~LOC | belongs / forbidden |
| --- | ---: | --- |
| `src/lib.rs` | 60 | `pub use r402_evm as evm`；`pub use r402_http as http`。再导出 `SettlementMode`。无类型定义。幸福路径 `r402::evm` / `r402::http::server`。 |

`Cargo.toml` 在 PR J：`default=["evm","http"]`；`http = ["dep:r402-http","r402-http/client","r402-http/server"]`（evm 同理）。空壳阶段 `default=[]`、无 path dep（K28）。

---

## `crates/r402-contract` — 3 `.rs` unpublished

Belongs：读 `tests/fixtures/contract/*.json`，断言键与数字。Forbidden：任何 `r402-*` 依赖、Axum、旧类型、`#[ignore]`。不进 `publish.yml`。

| Path | ~LOC | belongs |
| --- | ---: | --- |
| `src/lib.rs` | 40 | `//!` + `crate_loads` + JSON loader |
| `tests/headers.rs` | 80 | 断言 `headers.json` |
| `tests/modes.rs` | 80 | 断言 `settlement_modes.json` |

---

## `crates/r402-protocol` — 27 `.rs`

Forbidden：tokio、HTTP、链 RPC、`wire/`、`ChainRegistry`。

| Path | ~LOC | belongs |
| --- | ---: | --- |
| `src/lib.rs` | 40 | mods + pub use |
| `src/money.rs` | 250 | `MoneyAmount` |
| `src/metrics.rs` | 80 | `&str` 目录；`r402_background_settle_total` |
| `src/error.rs` | 200 | Verification/Settlement/Facilitator/ClientError；`Transport { kind: FacilitatorTransportKind }` |
| `src/error/reason.rs` | 250 | ErrorReason |
| `src/error/problem.rs` | 150 | PaymentProblem |
| `src/network.rs` | 80 | ChainProvider |
| `src/network/id.rs` | 150 | ChainId |
| `src/network/pattern.rs` | 150 | ChainIdPattern |
| `src/network/token.rs` | 120 | DeployedTokenAmount |
| `src/scheme.rs` | 200 | scheme markers；无 Sealed / SchemeBuilder |
| `src/extension.rs` | 350 | Extension trait + registry |
| `src/payment.rs` | 40 | mods |
| `src/payment/codec.rs` | 270 | Version2, Base64Bytes, U64String, UnixTimestamp |
| `src/payment/extensions.rs` | 220 | Extensions map |
| `src/payment/resource.rs` | 150 | ResourceInfo |
| `src/payment/requirements.rs` | 250 | PaymentRequirements + matching |
| `src/payment/required.rs` | 120 | PaymentRequired |
| `src/payment/payload.rs` | 150 | PaymentPayload |
| `src/payment/price.rs` | 80 | PriceTag |
| `src/payment/verify.rs` | 350 | VerifyRequest/Response；变体上 `extension_responses` |
| `src/payment/settle.rs` | 350 | SettleRequest/Response；同上 |
| `src/payment/supported.rs` | 150 | extensions JSON **array** |
| `src/payment/overrides.rs` | 120 | SettlementOverrides |
| `tests/public_api.rs` | 200 | 无 `X-PAYMENT` |
| `tests/spec_fixtures.rs` | 170 | 读 spec_v2 JSON |
| `tests/payment_proptest.rs` | 370 | |

`FacilitatorTransportKind`：`Timeout` / `HttpStatus { status: u16 }` / `MalformedSuccessBody`。度量名 `r402_background_settle_total`。

---

## `crates/r402-client` — 9 `.rs`

Forbidden：HTTP、Axum、crate-root `client.rs`。

| Path | ~LOC | belongs |
| --- | ---: | --- |
| `src/lib.rs` | 30 | |
| `src/register.rs` | 250 | PaymentClient builder |
| `src/select.rs` | 200 | FirstMatch |
| `src/policy.rs` | 150 | PaymentPolicy |
| `src/spend.rs` | 250 | SpendControls 默认 $1 |
| `src/hooks.rs` | 200 | ClientHooks |
| `src/candidate.rs` | 200 | SchemeClient + PaymentCandidate |
| `tests/spend.rs` | 200 | |
| `tests/select.rs` | 150 | |

---

## `crates/r402-server` — 19 `.rs`

Forbidden：reqwest、Axum、`resource_server/`、`settlement/{sequential,concurrent,background,compatibility}.rs`。

| Path | ~LOC | belongs |
| --- | ---: | --- |
| `src/lib.rs` | 40 | `pub use settlement::SettlementMode` |
| `src/resource.rs` | 350 | ResourceServer builder |
| `src/verify.rs` | 300 | verify_payment |
| `src/settle.rs` | 350 | before/after/cancel phases。upfront AfterHandler no-op。escrow 两次 settle |
| `src/required.rs` | 250 | PaymentRequired build |
| `src/matching.rs` | 200 | find_matching_requirements |
| `src/cancel.rs` | 200 | CancellationGuard |
| `src/scheme.rs` | 200 | SchemeNetworkServer |
| `src/payment_flow.rs` | 250 | PaymentFlowName + phases |
| `src/payment_flow/extra.rs` | 250 | extra.paymentFlow / ATM（双概念，保留单孩子） |
| `src/settlement.rs` | 250 | `schedule` + `finish` + `run` + AfterHandler + ScheduledSettlement。兼容性函数住这里 |
| `src/settlement/tracker.rs` | 180 | BackgroundSettlementTracker |
| `src/hooks.rs` | 40 | mods |
| `src/hooks/resource.rs` | 250 | ResourceServerHooks |
| `src/hooks/policy.rs` | 350 | additive extra |
| `src/hooks/snapshot.rs` | 250 | settle-core snapshot |
| `tests/payment_flow.rs` | 200 | |
| `tests/settlement.rs` | 250 | finish vs run；upfront 一次 settle；escrow 两次 |
| `tests/hooks.rs` | 200 | |

`settlement.rs` API（无 Axum）：

```rust
pub fn schedule(flow: PaymentFlowPhases, mode: SettlementMode)
    -> Result<SettlementSchedule, IncompatibleSettlementMode>;

pub enum AfterHandler { WaitThenSettle, JoinSettle, SpawnSettle, EchoReceipt }

pub async fn finish(
    schedule: SettlementSchedule,
    settle: Option<impl Future<Output = Result<SettleResponse, FacilitatorError>> + Send>,
) -> Result<SettleResponse, FacilitatorError>;

pub async fn run<T, E>(
    schedule: SettlementSchedule,
    handler: impl Future<Output = Result<T, E>> + Send,
    settle: impl Future<Output = Result<SettleResponse, FacilitatorError>> + Send,
) -> ScheduledSettlement<T, E>;

pub enum ScheduledSettlement<T, E> {
    HandlerOkSettleOk { value: T, receipt: SettleResponse },
    HandlerErrDetach { error: E },
    SettleErr { value: T, error: FacilitatorError },
    Spawned { value: T },
}
```

禁止用一个 `run(handler, settle)` 覆盖 Sequential。`run` 不读 HTTP status、不读 overrides。

---

## `crates/r402-facilitator` — 10 `.rs`

| Path | ~LOC | belongs |
| --- | ---: | --- |
| `src/lib.rs` | 30 | |
| `src/facilitator.rs` | 250 | AFIT + DynFacilitator |
| `src/hooks.rs` | 250 | FacilitatorHooks |
| `src/cache.rs` | 250 | TtlSet |
| `src/pending.rs` | 200 | PendingSettlementStore |
| `src/http.rs` | 40 | feature http-client |
| `src/http/client.rs` | 320 | /verify /settle /supported；EXTENSION-RESPONSES |
| `src/http/auth.rs` | 150 | looksFlat |
| `src/http/retry.rs` | 120 | GET /supported 429 only |
| `tests/http.rs` | 250 | wiremock |

Transport 三态见 K12。GET `/supported` 仅 429 重试。

---

## `crates/r402-http` — 27 `.rs`

Forbidden：`paygate.rs`、`middleware.rs`、`cors.rs`、`cache.rs`、`X402Route`、`into_layer`。`server.rs` **必须** `pub use SettlementMode`。

HTTP **形状 B：**

```text
X402Middleware::try_new(url)?
    .with_base_url(origin)             // 必填；缺则 with_price_tag 失败
    .with_scheme(pattern, Eip155Exact)
    .with_auth_only(siwx_cfg)          // 可选；在 Middleware 上
    .with_price_tag(tag)?              // Result<X402Layer, BuildError::MissingBaseUrl>
        .with_settlement_mode(mode)
        .with_resource(url)            // 钉死完整 resource URL；不豁免 base_url
        .with_description(s)
        .with_mime_type(s)
        .with_hooks(h)
// tower::Layer 只在 X402Layer。无 into_layer。无 X402Route。
```

幸福路径：`.layer(x402.with_price_tag(...)?.with_settlement_mode(...))`。`with_resource` 覆盖该路由的 `ResourceInfo.url`，**不**替代 `with_base_url`。未调 `with_base_url` 时 `with_price_tag` 仍返回 `Err(MissingBaseUrl)`。永不读 `Host`，永不 `http://localhost`。

`Eip155Exact::price_tag(pay_to, asset, None)` 三参保持。

| Path | ~LOC | belongs |
| --- | ---: | --- |
| `src/lib.rs` | 40 | 再导出 X402Client, WithPayments, X402Middleware, SettlementMode |
| `src/headers.rs` | 120 | Payment-*、SIGN-IN-WITH-X、`X402_EXPOSED_HEADERS`、`set_no_store`、`merge_private` |
| `src/buyer.rs` | 40 | feature client |
| `src/buyer/retry.rs` | 250 | 仅 402 重试 |
| `src/buyer/signature.rs` | 80 | Payment-Signature 头 |
| `src/server.rs` | 40 | feature server；`pub use SettlementMode` |
| `src/server/builder.rs` | 250 | X402Middleware；`with_base_url`；`with_auth_only`；`with_price_tag` → `Result<X402Layer, BuildError>`。声明 feature `client`/`server`（空壳无此 feature；G 起有） |
| `src/server/layer.rs` | 250 | X402Layer：`with_settlement_mode` / `with_resource` / `with_description` / `with_mime_type` / `with_hooks`。Layer impl。空 tag bypass。`with_resource` 不豁免 `base_url` |
| `src/server/gate.rs` | 250 | 三入口薄分发。`Gate`/`GateError`/`GateHooks` |
| `src/server/verify.rs` | 220 | 步骤 1–5 |
| `src/server/fail.rs` | 150 | 步骤 7–8；Transport 502 |
| `src/server/receipt.rs` | 150 | 步骤 11–12 |
| `src/server/pricing.rs` | 130 | PriceTagSource |
| `src/server/hooks.rs` | 100 | GrantAccess 槽 |
| `src/server/overrides.rs` | 250 | Settlement-Overrides 头 + UptoActualAmount |
| `src/server/siwx.rs` | 200 | feature siwx；实现 GrantAccess |
| `tests/sequential.rs` | 200 | |
| `tests/concurrent.rs` | 200 | overrides → 402 |
| `tests/background.rs` | 200 | strip；无 Payment-Response |
| `tests/headers.rs` | 150 | 头名 + Cache-Control + CORS |
| `tests/sidechannel.rs` | 120 | EXTENSION-RESPONSES 不进买家 |
| `tests/boundary.rs` | 150 | 502 vs 402 vs 500 |
| `tests/overrides.rs` | 150 | Sequential actual |
| `tests/siwx.rs` | 200 | 零 `/verify`；永不 Host |
| `tests/upfront.rs` | 120 | 一次 `/settle` |
| `tests/build.rs` | 80 | `with_price_tag` → MissingBaseUrl |
| `tests/buyer.rs` | 150 | 402 重试；412/502 不重试 |

16 src + 11 tests = **27**。无 `tests/cache.rs`。

### Gate 共享步骤（三入口；Concurrent/Background PR 不得复制 Sequential verify）

**Sequential `handle_request`：**

1. **GateHooks GrantAccess**（HTTP Sequential PR = 槽；SIWX PR 的 `siwx.rs` 实现）。空 tag bypass 在 `layer.rs`/`builder.rs`，不在 Gate。
2. compatibility（Concurrent/Background × upfront/escrow = 500）
3. ResourceServer verify
4. skip_handler → inline settle
5. settle-before if `phases.settle_before_handler`（upfront/escrow deposit）
6. call inner
7. `Ok(Response)` 4xx/5xx 当失败。**永不进 `run()`/`finish()`。**
8. 失败 → cancel-settle + failure-path Payment-Response；return
9. Sequential 成功 → 应用 Settlement-Overrides / UptoActualAmount
10. after-handler：`settle_after_handler` → `finish(WaitThenSettle, settle_fut)`；upfront → `finish(EchoReceipt, None)`；**escrow → `finish(WaitThenSettle, 第二次 settle)`**
11. 贴 Payment-Response iff Attach
12. Cache-Control（`set_no_store` / `merge_private`）；Transport → 502；isValid:false → 402；MissingScheme → 500

**Concurrent `handle_request_concurrent`：** 步骤 1–5 同（`verify.rs`）。然后：

6. 包装 inner（**调用 `run` 之前**）：`Ok(Response)` 4xx/5xx → `Err`；含 overrides/`UptoActualAmount` → `Err(SettlementAborted)`。未 poll 的 settle 按签名 max。
7. `run(JoinSettle, handler, settle)`。handler `Err` → `HandlerErrDetach` 并 **drop settle、不 await**。
8. `HandlerErrDetach(SettlementAborted)` → **402**；其它 `HandlerErrDetach` → cancel + 错误响应；`SettleErr` → 402；`HandlerOkSettleOk` → 步骤 11–12。

**Background `handle_request_background`：** 步骤 1–5 同。然后：

6. 包装 inner：仅 4xx/5xx → `Err`。不把 overrides 变 Err。settle 按签名 max。
7. `run(SpawnSettle, …)` → `Spawned { value: Response }`
8. 在该 Response 上 strip overrides/`UptoActualAmount`；不因此 402。
9. 不贴 Payment-Response。cache：无 receipt 不强制 private。

畸形 Payment-Signature → 402。

---

## `crates/r402-mcp` — 9 `.rs`

| Path | ~LOC | belongs |
| --- | ---: | --- |
| `src/lib.rs` | 30 | |
| `src/meta.rs` | 80 | `_meta` keys + JSON-RPC codes |
| `src/tool.rs` | 400 | dual-format tool result |
| `src/error.rs` | 170 | |
| `src/buyer.rs` | 400 | X402McpClient |
| `src/wrapper.rs` | 400 | PaymentWrapper |
| `src/wrapper/flow.rs` | 400 | paymentFlow only；Transport → tool isError |
| `tests/tool.rs` | 150 | |
| `tests/wrapper.rs` | 200 | |

---

## `crates/r402-extensions` — 12 `.rs`

| Path | ~LOC | belongs |
| --- | ---: | --- |
| `src/lib.rs` | 40 | |
| `src/bazaar.rs` | 130 | advertise-only |
| `src/payment_id.rs` | 320 | |
| `src/eip2612.rs` | 100 | |
| `src/erc20.rs` | 100 | |
| `src/siwx.rs` | 40 | feature siwx |
| `src/siwx/advertise.rs` | 150 | |
| `src/siwx/proof.rs` | 120 | |
| `src/siwx/origin.rs` | 80 | 永不 Host |
| `src/siwx/paid.rs` | 200 | PaidAddressStore |
| `tests/bazaar.rs` | 80 | |
| `tests/siwx.rs` | 200 | |

---

## `crates/r402-evm` — 57 `.rs`  **upto KEEP**

| Path | ~LOC | belongs |
| --- | ---: | --- |
| `src/lib.rs` | 80 | |
| `src/error.rs` | 100 | |
| `src/asset.rs` | 100 | |
| `src/permit2.rs` | 150 | |
| `src/signature.rs` | 130 | |
| `src/signer.rs` | 50 | |
| `src/eip2612.rs` | 180 | |
| `src/erc20.rs` | 120 | |
| `src/networks.rs` | 80 | |
| `src/networks/table.rs` | 250 | |
| `src/networks/usdc.rs` | 250 | |
| `src/chain/mod.rs` | 40 | |
| `src/chain/account.rs` | 250 | 今日 types.rs |
| `src/chain/provider.rs` | 300 | |
| `src/chain/nonce.rs` | 100 | |
| `src/chain/contracts.rs` | 40 | |
| `src/exact/mod.rs` | 60 | |
| `src/exact/payload.rs` | 360 | |
| `src/exact/client.rs` | 350 | |
| `src/exact/client/permit2.rs` | 220 | SPLIT of 569（双概念，保留） |
| `src/exact/server.rs` | 90 | **`price_tag(pay_to, asset, transfer_method: Option<AssetTransferMethod>)` 三参** |
| `src/exact/facilitator/mod.rs` | 350 | |
| `src/exact/facilitator/verify.rs` | 400 | |
| `src/exact/facilitator/settle.rs` | 370 | |
| `src/exact/facilitator/contract.rs` | 90 | |
| `src/exact/facilitator/signature.rs` | 50 | |
| `src/upto/mod.rs` | 100 | KEEP |
| `src/upto/payload.rs` | 310 | |
| `src/upto/server.rs` | 330 | |
| `src/upto/client.rs` | 280 | |
| `src/upto/client/signing.rs` | 170 | |
| `src/upto/client/eip2612.rs` | 180 | |
| `src/upto/facilitator/mod.rs` | 250 | |
| `src/upto/facilitator/verify.rs` | 350 | SPLIT of 1017 |
| `src/upto/facilitator/verify_witness.rs` | 350 | |
| `src/upto/facilitator/verify_permit2.rs` | 320 | |
| `src/upto/facilitator/settle.rs` | 150 | |
| `src/upto/facilitator/contract.rs` | 110 | |
| `src/auth_capture/mod.rs` | 150 | |
| `src/auth_capture/payload.rs` | 200 | |
| `src/auth_capture/client.rs` | 250 | |
| `src/auth_capture/server.rs` | 100 | |
| `src/auth_capture/facilitator.rs` | 200 | |
| `src/auth_capture/verify.rs` | 200 | |
| `src/auth_capture/nonce.rs` | 100 | |
| `src/batch_settlement/mod.rs` | 120 | |
| `src/batch_settlement/payload.rs` | 220 | |
| `src/batch_settlement/client.rs` | 180 | |
| `src/batch_settlement/server.rs` | 90 | |
| `src/batch_settlement/facilitator.rs` | 160 | |
| `src/batch_settlement/verify.rs` | 100 | |
| `src/batch_settlement/channel.rs` | 130 | |
| `src/batch_settlement/store.rs` | 120 | |
| `src/batch_settlement/voucher.rs` | 150 | |
| `tests/exact.rs` | 200 | |
| `tests/upto.rs` | 200 | |
| `tests/onchain_deployment.rs` | 226 | `R402_RPC_URLS` |

---

## `crates/r402-solana` — 41 `.rs`  **upto KEEP**

Forbidden：`shared.rs`、`payment_channels/`。

| Path | ~LOC | belongs |
| --- | ---: | --- |
| `src/lib.rs` | 70 | |
| `src/networks.rs` | 80 | |
| `src/networks/table.rs` | 250 | |
| `src/networks/usdc.rs` | 200 | |
| `src/chain/mod.rs` | 30 | |
| `src/chain/account.rs` | 250 | |
| `src/chain/provider.rs` | 300 | |
| `src/chain/rpc.rs` | 90 | |
| `src/exact/mod.rs` | 50 | |
| `src/exact/payload.rs` | 250 | |
| `src/exact/error.rs` | 120 | |
| `src/exact/client.rs` | 300 | |
| `src/exact/server.rs` | 130 | |
| `src/exact/facilitator/mod.rs` | 180 | |
| `src/exact/facilitator/verify.rs` | 350 | |
| `src/exact/facilitator/settle.rs` | 200 | 从今日 inlined settle 抽出 |
| `src/exact/facilitator/config.rs` | 110 | |
| `src/exact/facilitator/memo.rs` | 110 | |
| `src/exact/facilitator/smart_wallet.rs` | 250 | |
| `src/upto/mod.rs` | 60 | KEEP |
| `src/upto/payload.rs` | 110 | |
| `src/upto/error.rs` | 70 | |
| `src/upto/client.rs` | 260 | |
| `src/upto/server.rs` | 350 | |
| `src/upto/server/escrow.rs` | 330 | SPLIT of 679（双概念，保留） |
| `src/upto/facilitator/mod.rs` | 200 | |
| `src/upto/facilitator/verify.rs` | 250 | |
| `src/upto/facilitator/settle.rs` | 300 | |
| `src/upto/facilitator/settle/distribute.rs` | 300 | SPLIT of 607（双概念，保留） |
| `src/upto/facilitator/config.rs` | 80 | |
| `src/upto/facilitator/storage.rs` | 80 | |
| `src/upto/channel/mod.rs` | 50 | 今日 payment_channels |
| `src/upto/channel/account.rs` | 170 | |
| `src/upto/channel/codec.rs` | 200 | |
| `src/upto/channel/instructions.rs` | 220 | |
| `src/upto/channel/open.rs` | 350 | |
| `src/upto/channel/open/accounts.rs` | 350 | SPLIT of 713（双概念，保留） |
| `src/upto/channel/program.rs` | 140 | |
| `src/upto/channel/voucher.rs` | 90 | |
| `tests/exact.rs` | 200 | |
| `tests/upto.rs` | 200 | |

---

## Exact-only chains

features：`client, server, facilitator, telemetry, full`。Forbidden：`types.rs`、`hex.rs`、crate-root `provider.rs`、`codecs/`、上列 11 个装饰路径。`exact/client.rs` / `exact/facilitator/verify.rs`：400–600 豁免。

每个 crate 默认文件：`src/lib.rs`、`src/networks.rs`、`src/chain/mod.rs`、`src/chain/account.rs`、`src/exact/{mod,payload,error,client,server}.rs`、`src/exact/facilitator/{mod,verify}.rs`、`tests/exact.rs`。有 RPC 的加 `src/chain/rpc.rs`（单文件）。有 provider 的加 `src/chain/provider.rs`。有 settle 的加 `facilitator/settle.rs`（Casper 无）。

### `r402-algorand` — 18

| Path | ~LOC |
| --- | ---: |
| `src/lib.rs` | 70 |
| `src/networks.rs` | 150 |
| `src/chain/mod.rs` | 40 |
| `src/chain/account.rs` | 450 |
| `src/chain/provider.rs` | 120 |
| `src/chain/rpc.rs` | 580 |
| `src/chain/signer.rs` | 95 |
| `src/chain/codec.rs` | 350 |
| `src/chain/codec/group.rs` | 350 |
| `src/exact/mod.rs` | 40 |
| `src/exact/payload.rs` | 95 |
| `src/exact/error.rs` | 57 |
| `src/exact/client.rs` | 236 |
| `src/exact/server.rs` | 145 |
| `src/exact/facilitator/mod.rs` | 196 |
| `src/exact/facilitator/verify.rs` | 376 |
| `src/exact/facilitator/settle.rs` | 161 |
| `tests/exact.rs` | 400 |

KEEP `codec/group.rs`（God 814）。无 `account/asa.rs`。无 `rpc/algod.rs`。夹具 `spec_payment_group.json`。

### `r402-aptos` — 15

| Path | ~LOC |
| --- | ---: |
| `src/lib.rs` | 86 |
| `src/networks.rs` | 151 |
| `src/chain/mod.rs` | 22 |
| `src/chain/account.rs` | 420 |
| `src/chain/provider.rs` | 330 |
| `src/chain/codec.rs` | 560 |
| `src/exact/mod.rs` | 39 |
| `src/exact/payload.rs` | 85 |
| `src/exact/error.rs` | 53 |
| `src/exact/client.rs` | 388 |
| `src/exact/server.rs` | 144 |
| `src/exact/facilitator/mod.rs` | 243 |
| `src/exact/facilitator/verify.rs` | 331 |
| `src/exact/facilitator/settle.rs` | 105 |
| `tests/exact.rs` | 350 |

无 `account/fa.rs`。无 `codec/bcs.rs`。

### `r402-casper` — 17

| Path | ~LOC |
| --- | ---: |
| `src/lib.rs` | 95 |
| `src/networks.rs` | 199 |
| `src/chain/mod.rs` | 26 |
| `src/chain/motes.rs` | 350 |
| `src/chain/codec.rs` | 140 |
| `src/chain/account.rs` | 800 |
| `src/exact/mod.rs` | 88 |
| `src/exact/payload.rs` | 386 |
| `src/exact/error.rs` | 217 |
| `src/exact/eip712.rs` | 221 |
| `src/exact/client.rs` | 400 |
| `src/exact/server.rs` | 247 |
| `src/exact/facilitator/mod.rs` | 350 |
| `src/exact/facilitator/config.rs` | 281 |
| `src/exact/facilitator/transport.rs` | 109 |
| `src/exact/facilitator/verify.rs` | 400 |
| `tests/exact.rs` | 400 |

Forbidden：`hex.rs`。Env：`R402_CASPER_FACILITATOR_URL`。无 `settle.rs`。`account.rs` 对应 HEAD `types.rs` 854：TARGET **不**预声明 `account/key.rs`；PR Q 按 RULE 9 在填入时拆出有名字的第二概念文件（不改本表 17 / 合计 384）。

### `r402-hedera` — 16

| Path | ~LOC |
| --- | ---: |
| `src/lib.rs` | 59 |
| `src/networks.rs` | 206 |
| `src/chain/mod.rs` | 28 |
| `src/chain/account.rs` | 490 |
| `src/chain/provider.rs` | 257 |
| `src/chain/tx.rs` | 357 |
| `src/chain/rpc.rs` | 700 |
| `src/exact/mod.rs` | 39 |
| `src/exact/payload.rs` | 77 |
| `src/exact/error.rs` | 57 |
| `src/exact/client.rs` | 272 |
| `src/exact/server.rs` | 141 |
| `src/exact/facilitator/mod.rs` | 261 |
| `src/exact/facilitator/verify.rs` | 299 |
| `src/exact/facilitator/settle.rs` | 113 |
| `tests/exact.rs` | 400 |

无 `account/token.rs`。无 `rpc/mirror.rs`。

### `r402-keeta` — 15

| Path | ~LOC |
| --- | ---: |
| `src/lib.rs` | 48 |
| `src/networks.rs` | 162 |
| `src/chain/mod.rs` | 16 |
| `src/chain/account.rs` | 420 |
| `src/chain/provider.rs` | 269 |
| `src/exact/mod.rs` | 39 |
| `src/exact/payload.rs` | 110 |
| `src/exact/error.rs` | 44 |
| `src/exact/client.rs` | 312 |
| `src/exact/server.rs` | 104 |
| `src/exact/facilitator/mod.rs` | 147 |
| `src/exact/facilitator/verify.rs` | 252 |
| `src/exact/facilitator/settle.rs` | 115 |
| `src/exact/facilitator/queue.rs` | 300 |
| `tests/exact.rs` | 400 |

无 `account/id.rs`。

### `r402-near` — 15

| Path | ~LOC |
| --- | ---: |
| `src/lib.rs` | 79 |
| `src/networks.rs` | 153 |
| `src/chain/mod.rs` | 22 |
| `src/chain/account.rs` | 420 |
| `src/chain/provider.rs` | 295 |
| `src/chain/rpc.rs` | 570 |
| `src/exact/mod.rs` | 39 |
| `src/exact/payload.rs` | 71 |
| `src/exact/error.rs` | 44 |
| `src/exact/client.rs` | 273 |
| `src/exact/server.rs` | 93 |
| `src/exact/facilitator/mod.rs` | 167 |
| `src/exact/facilitator/verify.rs` | 291 |
| `src/exact/facilitator/settle.rs` | 133 |
| `tests/exact.rs` | 400 |

无 `account/nep.rs`。无 `rpc/query.rs`。

### `r402-stellar` — 17

| Path | ~LOC |
| --- | ---: |
| `src/lib.rs` | 92 |
| `src/networks.rs` | 167 |
| `src/chain/mod.rs` | 34 |
| `src/chain/account.rs` | 540 |
| `src/chain/provider.rs` | 308 |
| `src/chain/rpc.rs` | 490 |
| `src/chain/signer.rs` | 117 |
| `src/chain/xdr.rs` | 680 |
| `src/exact/mod.rs` | 39 |
| `src/exact/payload.rs` | 100 |
| `src/exact/error.rs` | 57 |
| `src/exact/client.rs` | 400 |
| `src/exact/server.rs` | 162 |
| `src/exact/facilitator/mod.rs` | 190 |
| `src/exact/facilitator/verify.rs` | 366 |
| `src/exact/facilitator/settle.rs` | 168 |
| `tests/exact.rs` | 400 |

无 `account/muxed.rs`。无 `rpc/simulate.rs`。无 `xdr/ops.rs`。xdr 单文件。

### `r402-tron` — 16

| Path | ~LOC |
| --- | ---: |
| `src/lib.rs` | 54 |
| `src/networks.rs` | 248 |
| `src/chain/mod.rs` | 31 |
| `src/chain/account.rs` | 510 |
| `src/chain/contracts.rs` | 66 |
| `src/chain/provider.rs` | 520 |
| `src/exact/mod.rs` | 56 |
| `src/exact/payload.rs` | 281 |
| `src/exact/error.rs` | 97 |
| `src/exact/client.rs` | 374 |
| `src/exact/server.rs` | 99 |
| `src/exact/facilitator/mod.rs` | 316 |
| `src/exact/facilitator/verify.rs` | 400 |
| `src/exact/facilitator/settle.rs` | 83 |
| `src/exact/facilitator/signature.rs` | 82 |
| `tests/exact.rs` | 200 |

无 `account/tip.rs`。无 `provider/grid.rs`。无 invented `facilitator/tests.rs`。

### `r402-tvm` — 24

| Path | ~LOC |
| --- | ---: |
| `src/lib.rs` | 80 |
| `src/networks.rs` | 146 |
| `src/chain/mod.rs` | 30 |
| `src/chain/account.rs` | 250 |
| `src/chain/provider.rs` | 300 |
| `src/chain/provider/toncenter.rs` | 300 |
| `src/chain/provider/tonapi.rs` | 300 |
| `src/chain/rpc.rs` | 198 |
| `src/chain/defaults.rs` | 80 |
| `src/chain/codec/mod.rs` | 20 |
| `src/chain/codec/cell.rs` | 146 |
| `src/chain/codec/w5.rs` | 400 |
| `src/chain/codec/highload_v3.rs` | 264 |
| `src/chain/codec/jetton.rs` | 173 |
| `src/exact/mod.rs` | 54 |
| `src/exact/payload.rs` | 156 |
| `src/exact/error.rs` | 127 |
| `src/exact/client.rs` | 400 |
| `src/exact/server.rs` | 134 |
| `src/exact/facilitator/mod.rs` | 209 |
| `src/exact/facilitator/verify.rs` | 400 |
| `src/exact/facilitator/settle.rs` | 107 |
| `src/exact/facilitator/batcher.rs` | 400 |
| `tests/exact.rs` | 350 |

Forbidden：crate-root `provider.rs`、`trace.rs`、`codecs/`。TVM `chain/codec/{cell,w5,highload_v3,jetton}` 与 `provider/{toncenter,tonapi}` 是多概念，保留。

### `r402-xrpl` — 16

| Path | ~LOC |
| --- | ---: |
| `src/lib.rs` | 98 |
| `src/networks.rs` | 207 |
| `src/chain/mod.rs` | 28 |
| `src/chain/account.rs` | 540 |
| `src/chain/codec.rs` | 237 |
| `src/chain/provider.rs` | 129 |
| `src/chain/rpc.rs` | 700 |
| `src/exact/mod.rs` | 38 |
| `src/exact/payload.rs` | 151 |
| `src/exact/error.rs` | 50 |
| `src/exact/client.rs` | 400 |
| `src/exact/server.rs` | 148 |
| `src/exact/facilitator/mod.rs` | 159 |
| `src/exact/facilitator/verify.rs` | 400 |
| `src/exact/facilitator/settle.rs` | 232 |
| `tests/exact.rs` | 400 |

无 `account/classic.rs`。无 `rpc/ledger.rs`。

---

## 不得存在

`crates/r402-core/`；`resource_server/`；`paygate.rs`；`middleware.rs`；`wire/`；`types.rs` 袋子；`shared.rs`；`X402Route`；`into_layer`；`vendor/`；11 个装饰路径；`settlement/{sequential,concurrent,background,compatibility}.rs`；`server/{cors,cache}.rs`。upto KEEP 故 `upto/` 必须存在于 evm/solana。
