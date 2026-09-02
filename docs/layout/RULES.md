# RULES — 代码必须住在哪里

闭合集 1–46。违规即拒。不添加兼容路径。

---

## Workspace

1. 根 `Cargo.toml` 只有 workspace。根上没有 `[package]`。
2. members 平铺 `crates/`。禁止 crate-in-crate。
3. `edition = "2024"`，`resolver = "3"`，`rust-version = "1.95"`（**因为 aptos-sdk，不是因为 qss/ccip 的 1.85**）。
4. binary：解析、调 library、打印。library `thiserror`；`anyhow` 只在 binary。
5. CI：`qntx/workflows/.../ci-rust.yml@v2`，`deny: true`，`apt-packages: protobuf-compiler`。Clippy：`cargo +nightly clippy --workspace --all-targets --all-features -- -D warnings`。rustfmt：`cargo +nightly fmt --all -- --config unstable_features=true,group_imports=StdExternalCrate,imports_granularity=Module`。`just all` = fmt + clippy-fix + deny + test + **doc-check**。
6. wipe 后只存在 TARGET 名字的 crate。禁止第二个 `r402-http`、`r402-*-legacy`、`reference/` members、与旧 `r402-core` 双编译。行为 oracle = tag `pre-wipe-0.17.1` **= `fb0e1cdfa4990a15cf6a78f7c5ee7e5c646f9b6a`**（git 操作，在任何 wipe commit 之前打）。

KEEP 空壳：`clippy.toml`、`rustfmt.toml`、`deny.toml`、`workspace.lints`、`.github/workflows/*`、`LICENSE-*`、`AGENTS.md`、`CONTRIBUTING.md`、`SECURITY.md`。`rust-toolchain.toml`：`channel=stable` + `components=["rustfmt","clippy"]` + `profile=minimal`。`Justfile` 保留 doc-check。**禁止加入 `vendor/`。不要 `git rm vendor/`。** HEAD 无此目录。

wipe **按目录** `git rm -r crates/r402 crates/r402-core …`（16 个旧名；WT 若已删则跳过）。禁止 `git rm crates/*`。空壳每个 `lib.rs`：`//!` + `crate_loads` 断言本包名。空壳 `Cargo.toml` 无其它 r402 path 依赖。`publish.yml` **不含** `r402-contract`。

---

## Crate placement

| 代码 | Crate |
| --- | --- |
| JSON 信封、CAIP-2、ErrorReason、scheme markers、金额、度量名字、ChainProvider、Extension trait、Transport | `r402-protocol` |
| Buyer register/select/spendControls | `r402-client` |
| Resource server、paymentFlow、SettlementMode scheduler、server hooks | `r402-server` |
| Facilitator trait、TtlSet、pending、远程 HTTP client、EXTENSION-RESPONSES | `r402-facilitator` |
| HTTP 头、Axum layer、reqwest buyer、CORS、Cache-Control、overrides 头、SIWX GrantAccess | `r402-http` |
| JSON 合同断言 | `r402-contract`（unpublished） |
| MCP | `r402-mcp` |
| 扩展实现 | `r402-extensions` |
| EIP-155 | `r402-evm` |
| Solana | `r402-solana` |
| 其它链 | `r402-{chain}` |
| re-export | `r402` |

依赖方向（禁止反转）：

```
client        → protocol
facilitator   → protocol
server        → protocol, facilitator
extensions    → protocol, facilitator
http          → protocol, client, server, facilitator, extensions
mcp           → protocol, client, server, facilitator
chain crates  → protocol, client, server, facilitator
umbrella      → 已启用 members
contract      → （无 r402 path 依赖）
```

链 crate 不得依赖 `r402-http`/`r402-mcp`。`r402-server`/`r402-facilitator` 不得依赖 `r402-extensions`。

---

## Module shape

7. `lib.rs`：只 `mod` 和 `pub use`。MCP keys 在 `meta.rs`。
8. `foo.rs` + `foo/` 仅当出现第二个文件。不为单孩子建目录。禁止预建 P0-3 的 11 个装饰路径。**禁止把 TARGET 强制孩子并回父文件**（`algorand/codec/group.rs` 必须存在）。实现 PR 可以 **新增** 孩子（God 填入时拆），不可删除强制行。
9. 一文件一概念。生产 ~300–400 行或第二概念 → 拆。TARGET **点名的** SPLIT 强制存在。豁免：exact-only `exact/client.rs` 与 `exact/facilitator/verify.rs` 改写后 400–600。RPC/codec >500 **可以**在实现 PR 拆，不强制 TARGET 行。God（≳800）**必须在该 crate 填入 PR 里**拆成有名字的第二概念模块。「必须拆」是填入义务，**不是** TARGET 预声明孩子的义务。未在设计期证明双概念的 God（Casper HEAD `types.rs` 854）TARGET 只列 `account.rs`；PR Q 拆出的文件不改 casper 17 / 合计 384。
10. 单元测试跟模块或 `tests/`。禁止 `src/**/facilitator/tests.rs`。
11. 两域共享 → 有名字的模块（`error`、`network`、`money`）。

---

## Banned names

永不创建：`utils`、`helpers`、`common`、`misc`、`data`、`tools`、`core2`、`lib2`、`functions`、`stuff`、`tmp`、`new_`、`old_`、`fix_`、`shared`。

永不作为文件/文件夹：`wire`、`paygate`、`middleware.rs`（类型 `X402Middleware` 允许）、`adapter.rs`、`types.rs` 袋子、`dyn_*.rs`、`encode.rs`、`r402-client` 根 `client.rs`、`resource_server/`、crate-root `hex.rs`/`trace.rs`、平级 `codecs/`。

禁止类型：`Paygate`、`PaygateHooks`、`PaygateError`、`X402Route`。用 `Gate`/`GateHooks`/`GateError`。无 `into_layer`。

度量禁止 `r402_paygate_*`。Background 计数器 `r402_background_settle_total`。

---

## Domain cuts inside a chain crate

```
src/chain/             primitives, RPC, codec, account — 无 scheme 名
src/exact/             仅 exact
src/upto/              仅 EVM、Solana。KEEP。
src/auth_capture/      仅 EVM
src/batch_settlement/  仅 EVM
src/networks/          仅当 >1 孩子（evm/solana 表）
```

Scheme 骨架：`exact/{mod,payload,error,client,server}.rs` + `facilitator/{mod,verify,settle}.rs`。

---

## Settlement modes vs paymentFlow

两域。禁止折叠。

| 名字 | Crate | 闭集 | 含义 |
| --- | --- | --- | --- |
| `PaymentFlowName` | `r402-server` | authorization / upfront / escrow | 链上价值相对 handler 何时移动 |
| `SettlementMode` | `r402-server` `src/settlement.rs` | Sequential / Concurrent / Background | authorization 的 after-handler settle，HTTP 何时等待 |

官方无 SettlementMode。三 mode KEEP。

12. Sequential/Concurrent/Background 代码住在 `settlement.rs` + `tracker.rs`，不在 HTTP gate 文件里。公开 API：`schedule` + `finish`（Sequential）+ `run`（Concurrent/Background）。无 Axum。
13. 两条调用点，禁止合成一个 `run` 给 Sequential。Sequential：Gate 跑 inner → 4xx 永不进 scheduler → 成功则 `finish`。Concurrent：Gate 先把 4xx/overrides 变成 Err，再 `run(JoinSettle)`；handler Err 立即 Detach 并 drop settle。Background：`run(SpawnSettle)` 后 strip。`run`/`finish` 不 verify、不 settle-before、不 cancel。Upfront Sequential：`EchoReceipt`，`finish(..., None)`。Escrow Sequential：settle-before + `finish(WaitThenSettle, 第二次 settle)`。MCP v1 不暴露 SettlementMode。
14. Concurrent/Background 对 upfront/escrow 非法。检查所有 resolved accepts。
15. actual 仅 Sequential。Concurrent：`run` 前标 Err(SettlementAborted)。Background：strip，不失败。
16. Background 不贴 Payment-Response。Concurrent handler 失败 detach。Sequential handler 失败 cancel。

---

## Plugins

无 `Plugin` 超 trait。无 `SettlementModePlugin`。无 `SchemeBuilder`/`SchemeRegistry`/`SchemeBlueprint`。`*Facilitator::try_new(provider)`。

17. 不要给 `SchemeClient` 加回 `Sealed`。
18. 不要加零调用者的 trait。DROP `ChainRegistry`。
19. SIWX：`siwx`/`siwx-evm`/`siwx-svm` 0.5。origin 必填。永不 Host。store key = configured origin + path。只 record Success。`with_auth_only`。feature `siwx`。v1 内存 store。challenge 在 402 extensions。错误码在 `r402-extensions::siwx`。
20. 不为 EIP-712/3009 依赖 `qntx/signer`。
21. 第三方链清单：`SchemeId` + `SchemeClient` + `SchemeNetworkServer` + `Facilitator` + `try_new`。

v1 不实现 builder-code / offer-receipt / http-message-signatures / auth-hints。未知 extension key roundtrip。

---

## Public API

22. 小公开面；其余 `pub(crate)`。
23. 公开函数返回 owned。
24. enum + newtype。禁止 `String`/`bool` 冒充 SettlementMode / PaymentFlowName。
25. Wire：`camelCase` + `deny_unknown_fields`。`GET /supported.extensions` 是 array。`extension_responses` 在变体上。
26. Umbrella 再导出幸福路径。只依赖 `r402` 时 `r402::evm` / `r402::http::server`。**HTTP 形状 B：** `with_price_tag` / `with_price_tags` / `with_dynamic_price` → `Result<X402Layer, BuildError>`。缺 `with_base_url` → `BuildError::MissingBaseUrl`（构造期，在这些方法里失败）。永不读 `Host`，永不 `http://localhost`。`tower::Layer` 只在 `X402Layer`。无 `X402Route`。无 `into_layer`。`X402Layer` 在 Ok 之后仍有 `with_settlement_mode` / `with_resource` / `with_description` / `with_mime_type` / `with_hooks`。**`with_resource` 不豁免 `base_url`。** `Eip155Exact::price_tag` 三参。消费方 binary 列出 anyhow/reqwest/alloy-signer-local。
27. ResourceServer builder：`with_scheme`/`register_scheme`、`with_hook`/`add_hook`、`with_extension`。
28. Umbrella feature 名保持 `ext-bazaar` / `ext-payment-id` / `ext-eip2612` / `ext-erc20-approval` / `all-extensions` / `cache`。加 `ext-siwx`。`default=["evm","http"]` 打开 client+server（PR J，Depends 含 G）。

---

## HTTP / MCP wire

29. 头：`Payment-Signature`、`SIGN-IN-WITH-X`、`Payment-Required`、`Payment-Response`。发出 Title-Case `Payment-*`。不把 SIWX / EXTENSION-RESPONSES 当买家响应头。Expose = `Payment-Required, Payment-Response`。Cache-Control：`set_no_store` / `merge_private`。
30. 状态：未付款/无效/畸形 → 402。Permit2 → 412。仅 Transport → 502。isValid false / scheme 失败 → 402。MissingScheme / IncompatibleSettlementMode / PaymentRequiredBuild → 500。Client 仅 402 重试。MCP 永不发 HTTP 502。
31. Facilitator：`POST /verify`、`POST /settle`、`GET /supported`。path-keyed auth。只对 `/supported` 429 重试。解析 EXTENSION-RESPONSES 到变体。
32. MCP：`_meta["x402/payment"]` / `_meta["x402/payment-response"]`；402 = tool isError dual format。Client 可解析 JSON-RPC 402 与 -32042；server 不发那些码。MCP SIWX v1 不做。

---

## Language / safety

33. 非测试：无 unwrap/expect/panic。例外：已证明 ASCII 的 header insert 可 expect+allow，或返回 Result。
34. `unsafe_code = "deny"`。
35. 无未使用的 trait 金字塔。无 SchemeBuilder。
36. Features：`client`/`server`/`facilitator`/`telemetry`/`cache`/`metrics`。`r402-http` 加 `siwx`。

---

## Tests

37. 夹具在 `tests/fixtures/`。`r402-contract` 只读 JSON，零 r402 类型，禁止 `#[ignore]`。行为 oracle = tag `pre-wipe-0.17.1`。
38. 新实现必须通过那些测试。禁止 copy 旧文件让测试绿。
39. `tests/fixtures/spec_v2/` 七个点名 JSON。round-trip bit-compatible。

---

## Docs / names

40. 文件夹名是领域名词。
41. README 幸福路径：`.with_price_tag(...)?`，无 `into_layer`。
42. 不保留旧模块路径兼容。
43. 禁止 `git mv` 旧文件进 TARGET。REWRITE。
44. 空壳 PR 的 `src/` 只有 `lib.rs`。出现 `money.rs` 即拒。
45. `r402-contract` 不出现在 `publish.yml`。
46. CI 不得把 `.rs` 计数当作门。
