# NAMES — 文件与文件夹法

crate 切分不够。文件名必须短名词、领域文件夹、没有袋子。对照：`/Users/xu/Desktop/qntx/qss`（2 crate / 30 `.rs` / 6 846 LOC）。多链对照是 `ovo-*` / `signer-*`。crate 名保持 `r402-*`。

规范角色名词钉在 `/Users/xu/Desktop/x/x402` @ `0344bdf54bf962e758480e76088eb2dc3209a169`。官方 TS 用 `client` / `server` / `facilitator` 三个角色文件夹；Rust 在 **chain crate 的 scheme 下** 沿用这三个词。Rust **不**抄 `utils.ts` / `types.ts` / `index.ts`。

公开 **类型名**（`X402Middleware`、`ResourceServer`、`PaymentClient`、`SettlementMode`、`X402Layer`）保留。它们背后的 **文件名** 全部按本表。禁止类型 `X402Route`。禁止方法 `into_layer`。

---

## qss 为什么「简洁优雅」（必须学）

`/Users/xu/Desktop/qntx/qss`：2 个 crate（`qss`、`qss-ecdsa`），**30** 个 `.rs`，**6 846** 行。

| 层 | qss 实际 |
| --- | --- |
| crate | 产品名词。`qss-ecdsa` 是一条签名路径，不是 `qss-core`。 |
| `src/` 文件夹 | 领域：`frost/`、`frost_tr/`、`chains/`。删 FROST = 删一个目录。 |
| 文件 | 短名词/动词：`dkg.rs`、`sign.rs`、`share.rs`、`id.rs`、`policy.rs`、`error.rs`。 |
| `lib.rs` | 只 `mod` + `pub use`。业务不进根。 |
| 不存在 | `utils`、`types.rs` 袋子、`common`、`helpers`、框架名（middleware）、内部品牌。 |
| 形状 | `foo.rs` + `foo/` 仅当出现第二个文件。不为单孩子建目录。 |

r402 会比 qss 多文件，因为有 12 条链。多出来的必须是 **链 × scheme × 角色**，不是袋子，也不是装饰 SPLIT。

ccip（空壳对照）：1 crate、1 个 `lib.rs`、`crate_loads` 测试。wipe 之后每个 TARGET crate 先长成这样，再自底向上填。

---

## Keep（名字 **就是** 领域）

| 名字 | 为什么 |
| --- | --- |
| `exact/` `upto/` `auth_capture/` `batch_settlement/` | spec scheme id；官方 TS `mechanisms/*/src/` 同名。 |
| `exact/{client,server,facilitator}/` | spec 三角色。官方 TS 同三个文件夹。 |
| `settlement.rs` + `settlement/tracker.rs` | 三 SettlementMode。官方 TS 没有。兼容性函数住在 `settlement.rs`。无 `settlement/{sequential,concurrent,background,compatibility}.rs`。 |
| `payment_flow/` | spec `extra.paymentFlow`。`extra.rs` 是双概念，不是装饰。 |
| `siwx/` `bazaar/` `payment_id/` | extension id。 |
| `headers.rs` | HTTP 头名 + CORS expose + Cache-Control（`set_no_store` / `merge_private`）。无独立 `cors.rs` / `cache.rs`。 |
| `error.rs` | 错误类型。 |
| `hooks.rs` | spec hook 点。 |
| `lib.rs` | crate 根。只 mods/`pub use`。 |
| crate 名 `r402-http` `r402-evm` `r402-mcp` `r402-protocol` … | 传输 / 链 / 产品名词。 |

`upto/` **KEEP**（EVM Permit2 + Solana escrow）。其它链不发明 `upto/`。DROP upto 的动作是删目录，不是改名。

**保留的单孩子**（双概念，不是装饰）：`payment_flow/extra.rs`、`wrapper/flow.rs`、`exact/client/permit2.rs`、`upto/server/escrow.rs`、`settle/distribute.rs`、`channel/open/accounts.rs`、`algorand/chain/codec/group.rs`（God 814）。

---

## 死名（历史、框架、黑话、crate 回声）

| 禁止 | 用这个 | 旧名为什么死 |
| --- | --- | --- |
| `wire/` `wire.rs` | `r402-protocol/src/payment/` | serde 黑话。spec 信封是 Payment*。 |
| `amount.rs` | `money.rs` | 领域是金额解析；类型名仍是 `MoneyAmount`。 |
| protocol 里的 `chain/` | `network/` | CAIP-2 **network**。`chain/` 留给链 crate 的 RPC。 |
| `r402-client/src/client.rs` | `register.rs` 等 | crate 已经叫 client。 |
| `resource_server/` | flatten：`resource.rs` `verify.rs` `settle.rs` … | 回声旧 God 路径。 |
| `scheme_adapter.rs` | `scheme.rs` | adapter 是事故。 |
| `dyn_facilitator.rs` | 折进 `facilitator.rs` | `dyn_` 是实现伎俩。 |
| `r402-http/src/client.rs` | `buyer.rs` | `client` 过载。 |
| `paygate` `paygate.rs` `Paygate` | `gate.rs`；类型 `Gate` | r402 私有品牌。 |
| `PaygateHooks` `PaygateError` | `GateHooks` `GateError` | 同上。 |
| `middleware.rs` | `builder.rs` + `layer.rs` | Tower 框架名。类型 `X402Middleware` 可留。 |
| `X402Route` `into_layer` | `with_price_tag` → `Result<X402Layer, BuildError>` | HTTP shape B。缺 `base_url` 构造期失败。 |
| `encode.rs` | `tool.rs` | 说清楚编的是什么。 |
| `types.rs` | `payload.rs` / `account.rs` | 袋子。 |
| `payment_channels/` | `upto/channel/` | TS 包残留。 |
| `shared.rs` `hex.rs` `trace.rs` `codecs/` | 吸收进命名模块 | junk。 |
| `server/cors.rs` `server/cache.rs` `tests/cache.rs` | `headers.rs` + `tests/headers.rs` | 过薄；并入 headers。 |
| `settlement/{sequential,concurrent,background,compatibility}.rs` | `settlement.rs` | 装饰空文件。 |
| 装饰 `account/<noun>.rs` | 单文件 `account.rs` | 无第二概念合同。Casper God 在填入 PR 按 RULE 9 拆，不预建。 |
| 度量名 `r402_paygate_*` | `r402_background_settle_total` | 度量名也不许带 `paygate`。 |

禁止预建的 11 个装饰路径：`algorand/chain/account/asa.rs`、`aptos/chain/account/fa.rs`、`casper/chain/account/key.rs`、`hedera/chain/account/token.rs`、`keeta/chain/account/id.rs`、`near/chain/account/nep.rs`、`stellar/chain/account/muxed.rs`、`tron/chain/account/tip.rs`、`xrpl/chain/account/classic.rs`、`stellar/chain/rpc/simulate.rs`、`tron/chain/provider/grid.rs`。也不预建 `rpc/{algod,mirror,query,ledger}.rs`、`aptos/chain/codec/bcs.rs`、`stellar/chain/xdr/ops.rs`。

---

## 放置

1. **Crate** = 删这个就删掉该功能。
2. **`src/` 一层文件夹** = spec 角色或 scheme（`payment/`、`exact/`、`buyer/`、`server/`）。
3. **文件** = 一个概念。名字是概念（`gate.rs`、`money.rs`、`payload.rs`）。
4. **`foo.rs` + `foo/`** 仅当出现第二个文件。不为单孩子建目录。禁止预建上列 11 个装饰路径。
5. **`lib.rs`**：`mod` + `pub use`。常量进 `meta.rs` / 领域文件。
6. 链 crate：

```
src/
  lib.rs
  networks.rs
  chain/               RPC、codec、account — 从不出现 scheme 名
  exact/               仅 exact
    payload.rs
    error.rs
    client.rs          spec 角色
    server.rs
    facilitator/
      verify.rs
      settle.rs
  upto/                仅当该链有 upto（EVM、Solana）
```

7. HTTP：`buyer/` 是付钱的客户端；`server/` 是 Axum 适配；`server/gate.rs` 是 402 管道；`settlement` **不**住在 HTTP crate。`with_resource` 不豁免 `base_url`。

---

## 计数（src + tests `*.rs`，不含 JSON/b64 夹具）

实现 PR **可以多** SPLIT God（Casper `account.rs` 在 PR Q 拆出的孩子不算少行）。**不可少强制行。禁止把强制孩子并回父文件**（`algorand/codec/group.rs` 强制）。CI **不得** `wc -l == 384`。

wipe 后空壳是每 crate 1 个 `lib.rs`；下表是填满后的强制目标。

| Crate | `src/` 下文件夹（不含 `src` 本身） | `.rs`（src+tests） | 备注 |
| --- | ---: | ---: | --- |
| `r402` | 0 | 1 | umbrella |
| `r402-protocol` | `error` `network` `payment` | 27 | 无 `wire` |
| `r402-client` | 0 | 9 | 无 crate-root `client.rs` |
| `r402-server` | `payment_flow` `settlement` `hooks` | 19 | 无四 mode 装饰文件；`finish`+`run` 在 `settlement.rs` |
| `r402-facilitator` | `http` | 10 | 无 `dyn_facilitator.rs` |
| `r402-http` | `buyer` `server` | 27 | cors+cache 并入 `headers.rs`；shape B；11 tests |
| `r402-mcp` | `wrapper` | 9 | `tool.rs` / `meta.rs` / `buyer.rs` |
| `r402-extensions` | `siwx` | 12 | 一扩展一文件或一目录 |
| `r402-contract` | 0 | 3 | unpublished；JSON-only；无 r402 类型 |
| `r402-evm` | `networks` `chain` `exact` `upto` `auth_capture` `batch_settlement` + 子目录 | 57 | 含 `upto/` |
| `r402-solana` | `networks` `chain` `exact` `upto` `upto/channel` | 41 | `channel` 不是 `payment_channels` |
| `r402-algorand` | `chain` `exact` | 18 | KEEP `codec/group.rs`；无 `account/asa`、无 `rpc/algod` |
| `r402-aptos` | `chain` `exact` | 15 | 无 `account/fa`、无 `codec/bcs` |
| `r402-casper` | `chain` `exact` | 17 | 无 `hex.rs`；无 `account/key.rs`；无 settle.rs |
| `r402-hedera` | `chain` `exact` | 16 | 无 `account/token`、无 `rpc/mirror` |
| `r402-keeta` | `chain` `exact` | 15 | 无 `account/id` |
| `r402-near` | `chain` `exact` | 15 | 无 `account/nep`、无 `rpc/query` |
| `r402-stellar` | `chain` `exact` | 17 | 无 `account/muxed`、无 `rpc/simulate`、无 `xdr/ops` |
| `r402-tron` | `chain` `exact` | 16 | 无 `account/tip`、无 `provider/grid` |
| `r402-tvm` | `chain` `exact` | 24 | codec 在 chain/；无 crate-root provider |
| `r402-xrpl` | `chain` `exact` | 16 | 无 `account/classic`、无 `rpc/ledger` |
| **合计** | | **384** | 与 TARGET_TREE crate 表一致。今日 295；增长来自命名拆分 + SIWX + settlement + gate pipeline + contract crate。不是 408。 |

文件夹只在 ≥2 个孩子（或 scheme/角色目录）时存在。

---

## 官方 TS vs 本树（禁止 1:1 镜像）

| Official TS | Rust TARGET |
| --- | --- |
| `core/src/types/payments.ts` | `r402-protocol/src/payment/*.rs` |
| `core/src/client/x402Client.ts` | `r402-client/src/register.rs` + `select`/`spend` |
| `core/src/server/x402ResourceServer.ts` | `r402-server/src/resource.rs` |
| `core/src/server/paymentFlow.ts` | `r402-server/src/payment_flow.rs` |
| `core/src/http/httpFacilitatorClient.ts` | `r402-facilitator/src/http/client.rs` |
| `core/src/http/x402HTTPResourceServer.ts` | `r402-http/src/server/gate.rs` |
| `http/express/src/adapter.ts` | `r402-http/src/server/{builder,layer,gate}.rs`（Axum；无 Route） |
| `mechanisms/evm/src/exact/{client,server,facilitator}/` | `r402-evm/src/exact/{client,server,facilitator}/` **同名词** |
| `mechanisms/*/src/utils.ts` | **无对应物** |
| `mechanisms/*/src/types.ts` | `exact/payload.rs` |
| `mcp/src/utils/encoding.ts` | `r402-mcp/src/tool.rs` |

---

## 施工注记

wipe 之后旧路径不存在。禁止 `git mv paygate.rs gate.rs`。新文件是对着行为 + spec **重写**的。HEAD 无 `vendor/`；不要 `git rm vendor/`。
