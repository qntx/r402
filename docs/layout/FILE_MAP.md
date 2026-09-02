# FILE_MAP — 旧路径 → REWRITE / SPLIT / DROP

默认：**拒绝保留原路径**。动作不是 `git mv`。

- **REWRITE** — 旧文件的行为写进一个新文件。wipe 之后对着 spec + 旧行为重写。
- **SPLIT** — 一个旧文件的行为写进两个或更多新文件。每一行点名全部输出。
- **DROP** — 目标树不存在该文件。行为删除或吸收进命名模块。

**覆盖声明：** HEAD `fb0e1cd` 每个 `crates/**/*.rs`（295）∈ 下列 **点名行 ∪ named glob ∪ DROP**，余 0。named glob 的 `{a,b}` 按 Rust glob 展开（每个名字一文件）。夹具点名见根树 / TARGET_TREE。

**施工。** wipe **按目录** `git rm -r` 下列 16 个目录（**禁止** `git rm crates/*`）：`crates/r402` `crates/r402-core` `crates/r402-http` `crates/r402-mcp` `crates/r402-evm` `crates/r402-solana` `crates/r402-tron` `crates/r402-casper` `crates/r402-near` `crates/r402-xrpl` `crates/r402-hedera` `crates/r402-algorand` `crates/r402-aptos` `crates/r402-keeta` `crates/r402-tvm` `crates/r402-stellar`。**不要** `git rm vendor/`（HEAD 无此目录）。之后本表的「New」从空 `lib.rs` 长出来。旧路径在 tag `pre-wipe-0.17.1` = `fb0e1cd` 只读。

---

## Umbrella

| Old | Action | New |
| --- | --- | --- |
| `crates/r402/src/lib.rs` | REWRITE | `crates/r402/src/lib.rs`（不再 `pub use r402_core::*`） |

---

## `r402-core`（30，全点名）

| Old | Action | New |
| --- | --- | --- |
| `src/lib.rs` | DROP | |
| `src/amount.rs` | REWRITE | `r402-protocol/src/money.rs` |
| `src/cache.rs` | REWRITE | `r402-facilitator/src/cache.rs` |
| `src/chain.rs` | SPLIT | `r402-protocol/src/network.rs` + `network/{id,pattern,token}.rs`；DROP `ChainRegistry` |
| `src/client.rs` | SPLIT | `r402-client/src/{register,select,policy,spend,hooks,candidate}.rs` |
| `src/error.rs` | SPLIT | `r402-protocol/src/error.rs` + `error/{reason,problem}.rs` + Transport |
| `src/extensions/mod.rs` | REWRITE | `r402-protocol/src/extension.rs` |
| `src/extensions/bazaar.rs` | REWRITE | `r402-extensions/src/bazaar.rs` |
| `src/extensions/eip2612.rs` | REWRITE | `r402-extensions/src/eip2612.rs` |
| `src/extensions/erc20_approval.rs` | REWRITE | `r402-extensions/src/erc20.rs` |
| `src/extensions/payment_id.rs` | REWRITE | `r402-extensions/src/payment_id.rs` |
| `src/facilitator.rs` | SPLIT | `r402-facilitator/src/{facilitator,hooks}.rs` |
| `src/metrics.rs` | SPLIT | 名字 → protocol `metrics.rs`；`record_*` → 调用点 |
| `src/pending_settlement.rs` | REWRITE | `r402-facilitator/src/pending.rs` |
| `src/resource_server/mod.rs` | SPLIT | `r402-server/src/{resource,verify,settle,required,matching,cancel}.rs` |
| `src/resource_server/hook_policy.rs` | SPLIT | `r402-server/src/hooks/{policy,snapshot}.rs` |
| `src/resource_server/hooks.rs` | REWRITE | `r402-server/src/hooks/resource.rs` |
| `src/resource_server/payment_flow.rs` | SPLIT | `payment_flow.rs` + `payment_flow/extra.rs` |
| `src/resource_server/scheme.rs` | REWRITE | `r402-server/src/scheme.rs` |
| `src/scheme/mod.rs` | REWRITE | `r402-protocol/src/scheme.rs`；DROP `Sealed` |
| `src/scheme/client.rs` | SPLIT | `r402-client/src/{candidate,select}.rs` |
| `src/scheme/registry.rs` | DROP | |
| `src/wire/mod.rs` | DROP | |
| `src/wire/codec.rs` | REWRITE | `payment/codec.rs` |
| `src/wire/extensions.rs` | REWRITE | `payment/extensions.rs` |
| `src/wire/offer.rs` | SPLIT | `payment/{resource,requirements,required,payload,price}.rs` |
| `src/wire/rpc.rs` | SPLIT | `payment/{verify,settle,supported,overrides}.rs` |
| `tests/public_api.rs` | REWRITE | `r402-protocol/tests/public_api.rs` |
| `tests/spec_fixtures.rs` | REWRITE | 读 workspace spec_v2 |
| `tests/wire_proptest.rs` | REWRITE | `tests/payment_proptest.rs` |

---

## `r402-http`（10，全点名）

| Old | Action | New |
| --- | --- | --- |
| `src/lib.rs` | REWRITE | `src/lib.rs` |
| `src/client.rs` | SPLIT | `buyer.rs` + `buyer/{retry,signature}.rs` |
| `src/server/mod.rs` | REWRITE | `src/server.rs` |
| `src/server/paygate.rs` | SPLIT | `server/{gate,verify,fail,receipt}.rs` + `r402-server/src/settlement.rs` + `settlement/tracker.rs` |
| `src/server/middleware.rs` | SPLIT | `server/builder.rs` + `server/layer.rs`（无 Route、无 `into_layer`） |
| `src/server/facilitator.rs` | SPLIT | `r402-facilitator/src/http/{client,auth,retry}.rs` |
| `src/server/hooks.rs` | REWRITE | `server/hooks.rs` |
| `src/server/pricing.rs` | REWRITE | 同 |
| `src/server/tracker.rs` | REWRITE | `r402-server/src/settlement/tracker.rs` |
| `src/server/upto.rs` | REWRITE | `server/overrides.rs` |

新文件（无旧路径）：`src/headers.rs`（Payment-* + SIGN-IN-WITH-X + CORS expose + `set_no_store` / `merge_private`）；`src/server/siwx.rs`；`tests/{sequential,concurrent,background,headers,sidechannel,boundary,overrides,siwx,upfront,build,buyer}.rs`。无 `cors.rs` / `cache.rs` / `tests/cache.rs`。

---

## `r402-mcp`（5）

| Old | Action | New |
| --- | --- | --- |
| `src/lib.rs` | REWRITE | `src/lib.rs` |
| `src/encode.rs` | REWRITE | `src/tool.rs` |
| `src/error.rs` | REWRITE | `src/error.rs` |
| `src/client.rs` | REWRITE | `src/buyer.rs` |
| `src/server.rs` | SPLIT | `src/wrapper.rs` + `wrapper/flow.rs` |

常量 → `src/meta.rs`。

---

## `r402-evm` named glob（50 HEAD `.rs`）

| Glob / Old | Action | New |
| --- | --- | --- |
| `src/lib.rs` | REWRITE | 同 |
| `src/error.rs` | REWRITE | 同 |
| `src/asset.rs` | REWRITE | 同 |
| `src/permit2.rs` | REWRITE | 同 |
| `src/eip2612.rs` | REWRITE | 同 |
| `src/signature.rs` | REWRITE | 同 |
| `src/signer.rs` | REWRITE | 同 |
| `src/erc20_approval.rs` | REWRITE | `src/erc20.rs` |
| `src/networks.rs` | SPLIT | `networks.rs` + `networks/{table,usdc}.rs` |
| `src/chain/mod.rs` | REWRITE | 同 |
| `src/chain/provider.rs` | REWRITE | 同 |
| `src/chain/nonce.rs` | REWRITE | 同 |
| `src/chain/contracts.rs` | REWRITE | 同 |
| `src/chain/types.rs` | REWRITE | `src/chain/account.rs` |
| `src/exact/mod.rs` | REWRITE | 同 |
| `src/exact/client.rs` | SPLIT | `exact/client.rs` + `client/permit2.rs` |
| `src/exact/server.rs` | REWRITE | 同 |
| `src/exact/types.rs` | REWRITE | `exact/payload.rs` |
| `src/exact/facilitator/mod.rs` | REWRITE | 同 |
| `src/exact/facilitator/verify.rs` | REWRITE | 同 |
| `src/exact/facilitator/settle.rs` | REWRITE | 同 |
| `src/exact/facilitator/contract.rs` | REWRITE | 同 |
| `src/exact/facilitator/signature.rs` | REWRITE | 同 |
| `src/upto/mod.rs` | REWRITE | 同 |
| `src/upto/server.rs` | REWRITE | 同 |
| `src/upto/types.rs` | REWRITE | `upto/payload.rs` |
| `src/upto/client/mod.rs` | REWRITE | `upto/client.rs` |
| `src/upto/client/signing.rs` | REWRITE | `upto/client/signing.rs` |
| `src/upto/client/eip2612.rs` | REWRITE | `upto/client/eip2612.rs` |
| `src/upto/facilitator/mod.rs` | REWRITE | 同 |
| `src/upto/facilitator/settle.rs` | REWRITE | 同 |
| `src/upto/facilitator/contract.rs` | REWRITE | 同 |
| `src/upto/facilitator/verify.rs` | SPLIT | `verify.rs` + `verify_witness.rs` + `verify_permit2.rs` |
| `src/auth_capture/mod.rs` | REWRITE | 同 |
| `src/auth_capture/client.rs` | REWRITE | 同 |
| `src/auth_capture/server.rs` | REWRITE | 同 |
| `src/auth_capture/facilitator.rs` | REWRITE | 同 |
| `src/auth_capture/verify.rs` | REWRITE | 同 |
| `src/auth_capture/nonce.rs` | REWRITE | 同 |
| `src/auth_capture/types.rs` | REWRITE | `auth_capture/payload.rs` |
| `src/batch_settlement/mod.rs` | REWRITE | 同 |
| `src/batch_settlement/client.rs` | REWRITE | 同 |
| `src/batch_settlement/server.rs` | REWRITE | 同 |
| `src/batch_settlement/facilitator.rs` | REWRITE | 同 |
| `src/batch_settlement/verify.rs` | REWRITE | 同 |
| `src/batch_settlement/channel.rs` | REWRITE | 同 |
| `src/batch_settlement/store.rs` | REWRITE | 同 |
| `src/batch_settlement/voucher.rs` | REWRITE | 同 |
| `src/batch_settlement/types.rs` | REWRITE | `batch_settlement/payload.rs` |
| `tests/onchain_deployment.rs` | REWRITE | 同 |

新：`tests/exact.rs`、`tests/upto.rs`。

---

## `r402-solana` named glob（34 HEAD `.rs`）

| Glob / Old | Action | New |
| --- | --- | --- |
| `src/lib.rs` | REWRITE | 同 |
| `src/networks.rs` | SPLIT | `networks.rs` + `networks/{table,usdc}.rs` |
| `src/chain/mod.rs` | REWRITE | 同 |
| `src/chain/provider.rs` | REWRITE | 同 |
| `src/chain/rpc.rs` | REWRITE | 同 |
| `src/chain/types.rs` | REWRITE | `account.rs` |
| `src/exact/mod.rs` | REWRITE | 同 |
| `src/exact/error.rs` | REWRITE | 同 |
| `src/exact/client.rs` | REWRITE | 同 |
| `src/exact/server.rs` | REWRITE | 同 |
| `src/exact/types.rs` | REWRITE | `payload.rs` |
| `src/exact/facilitator/mod.rs` | SPLIT | `mod.rs` + 抽出 `settle.rs` |
| `src/exact/facilitator/verify.rs` | REWRITE | 同 |
| `src/exact/facilitator/config.rs` | REWRITE | 同 |
| `src/exact/facilitator/memo.rs` | REWRITE | 同 |
| `src/exact/facilitator/smart_wallet.rs` | REWRITE | 同 |
| `src/upto/mod.rs` | REWRITE | 同 |
| `src/upto/error.rs` | REWRITE | 同 |
| `src/upto/client.rs` | REWRITE | 同 |
| `src/upto/server.rs` | SPLIT | `upto/server.rs` + `server/escrow.rs` |
| `src/upto/types.rs` | REWRITE | `payload.rs` |
| `src/upto/shared.rs` | DROP | 吸收进 `upto/channel/{account,codec,voucher}.rs` |
| `src/upto/facilitator/mod.rs` | REWRITE | 同 |
| `src/upto/facilitator/verify.rs` | REWRITE | 同 |
| `src/upto/facilitator/settle.rs` | SPLIT | `settle.rs` + `settle/distribute.rs` |
| `src/upto/facilitator/config.rs` | REWRITE | 同 |
| `src/upto/facilitator/storage.rs` | REWRITE | 同 |
| `src/upto/payment_channels/mod.rs` | REWRITE | `upto/channel/mod.rs` |
| `src/upto/payment_channels/channel.rs` | REWRITE | `upto/channel/account.rs` |
| `src/upto/payment_channels/codec.rs` | REWRITE | `upto/channel/codec.rs` |
| `src/upto/payment_channels/instructions.rs` | REWRITE | `upto/channel/instructions.rs` |
| `src/upto/payment_channels/open.rs` | SPLIT | `upto/channel/open.rs` + `open/accounts.rs` |
| `src/upto/payment_channels/program.rs` | REWRITE | `upto/channel/program.rs` |
| `src/upto/payment_channels/voucher.rs` | REWRITE | `upto/channel/voucher.rs` |

新：`tests/exact.rs`、`tests/upto.rs`。

---

## Exact-only named rows

对 `{algorand,aptos,hedera,keeta,near,stellar,tron,xrpl}` 共同文件（Casper/TVM extras 另列）。**不**预建 `account/<noun>.rs`、`rpc/<svc>.rs`、`provider/grid.rs`、`codec/bcs.rs`、`xdr/ops.rs`。

### 共同骨架（每条链点名）

| Glob | Action | New |
| --- | --- | --- |
| `src/lib.rs` | REWRITE | 同 |
| `src/networks.rs` | REWRITE | 同 |
| `src/chain/mod.rs` | REWRITE | 同 |
| `src/chain/types.rs` | REWRITE | `src/chain/account.rs` |
| `src/chain/provider.rs` | REWRITE | 同（tron 不预建 `provider/grid.rs`） |
| `src/chain/rpc.rs` | REWRITE | 同（不预建 `rpc/<svc>.rs`） |
| `src/exact/mod.rs` | REWRITE | 同 |
| `src/exact/error.rs` | REWRITE | 同 |
| `src/exact/client.rs` | REWRITE | 同 |
| `src/exact/server.rs` | REWRITE | 同 |
| `src/exact/types.rs` | REWRITE | `payload.rs` |
| `src/exact/facilitator/mod.rs` | REWRITE | 同 |
| `src/exact/facilitator/verify.rs` | REWRITE | 同 |
| `src/exact/facilitator/settle.rs` | REWRITE | 同（casper 无此 HEAD 文件） |
| `src/exact/facilitator/tests.rs` | REWRITE | `tests/exact.rs`（tron 无此文件：skip） |

下列 crate 相对路径全部命中上表或 extras：

**algorand（17）：** `src/lib.rs` `src/networks.rs` `src/chain/mod.rs` `src/chain/types.rs` `src/chain/codec.rs` `src/chain/provider.rs` `src/chain/rpc.rs` `src/chain/signer.rs` `src/exact/mod.rs` `src/exact/types.rs` `src/exact/error.rs` `src/exact/client.rs` `src/exact/server.rs` `src/exact/facilitator/mod.rs` `src/exact/facilitator/verify.rs` `src/exact/facilitator/settle.rs` `src/exact/facilitator/tests.rs`

**aptos（15）：** `src/lib.rs` `src/networks.rs` `src/chain/mod.rs` `src/chain/types.rs` `src/chain/codec.rs` `src/chain/provider.rs` `src/exact/mod.rs` `src/exact/types.rs` `src/exact/error.rs` `src/exact/client.rs` `src/exact/server.rs` `src/exact/facilitator/mod.rs` `src/exact/facilitator/verify.rs` `src/exact/facilitator/settle.rs` `src/exact/facilitator/tests.rs`

**hedera（16）：** `src/lib.rs` `src/networks.rs` `src/chain/mod.rs` `src/chain/types.rs` `src/chain/provider.rs` `src/chain/rpc.rs` `src/chain/tx.rs` `src/exact/mod.rs` `src/exact/types.rs` `src/exact/error.rs` `src/exact/client.rs` `src/exact/server.rs` `src/exact/facilitator/mod.rs` `src/exact/facilitator/verify.rs` `src/exact/facilitator/settle.rs` `src/exact/facilitator/tests.rs`

**keeta（15）：** `src/lib.rs` `src/networks.rs` `src/chain/mod.rs` `src/chain/types.rs` `src/chain/provider.rs` `src/exact/mod.rs` `src/exact/types.rs` `src/exact/error.rs` `src/exact/client.rs` `src/exact/server.rs` `src/exact/facilitator/mod.rs` `src/exact/facilitator/verify.rs` `src/exact/facilitator/settle.rs` `src/exact/facilitator/queue.rs` `src/exact/facilitator/tests.rs`

**near（15）：** `src/lib.rs` `src/networks.rs` `src/chain/mod.rs` `src/chain/types.rs` `src/chain/provider.rs` `src/chain/rpc.rs` `src/exact/mod.rs` `src/exact/types.rs` `src/exact/error.rs` `src/exact/client.rs` `src/exact/server.rs` `src/exact/facilitator/mod.rs` `src/exact/facilitator/verify.rs` `src/exact/facilitator/settle.rs` `src/exact/facilitator/tests.rs`

**stellar（17）：** `src/lib.rs` `src/networks.rs` `src/chain/mod.rs` `src/chain/types.rs` `src/chain/provider.rs` `src/chain/rpc.rs` `src/chain/signer.rs` `src/chain/xdr.rs` `src/exact/mod.rs` `src/exact/types.rs` `src/exact/error.rs` `src/exact/client.rs` `src/exact/server.rs` `src/exact/facilitator/mod.rs` `src/exact/facilitator/verify.rs` `src/exact/facilitator/settle.rs` `src/exact/facilitator/tests.rs`

**tron（15）：** `src/lib.rs` `src/networks.rs` `src/chain/mod.rs` `src/chain/types.rs` `src/chain/provider.rs` `src/chain/contracts.rs` `src/exact/mod.rs` `src/exact/types.rs` `src/exact/error.rs` `src/exact/client.rs` `src/exact/server.rs` `src/exact/facilitator/mod.rs` `src/exact/facilitator/verify.rs` `src/exact/facilitator/settle.rs` `src/exact/facilitator/signature.rs`

**xrpl（16）：** `src/lib.rs` `src/networks.rs` `src/chain/mod.rs` `src/chain/types.rs` `src/chain/codec.rs` `src/chain/provider.rs` `src/chain/rpc.rs` `src/exact/mod.rs` `src/exact/types.rs` `src/exact/error.rs` `src/exact/client.rs` `src/exact/server.rs` `src/exact/facilitator/mod.rs` `src/exact/facilitator/verify.rs` `src/exact/facilitator/settle.rs` `src/exact/facilitator/tests.rs`

### Casper extras（16 HEAD `.rs`）

| Old | Action | New |
| --- | --- | --- |
| `src/lib.rs` | REWRITE | 同 |
| `src/networks.rs` | REWRITE | 同 |
| `src/hex.rs` | DROP | 逻辑 → `chain/codec.rs` |
| `src/motes.rs` | REWRITE | `chain/motes.rs` |
| `src/chain/mod.rs` | REWRITE | 同 |
| `src/chain/types.rs` | REWRITE | `src/chain/account.rs`（**不**预建 `account/key.rs`） |
| `src/exact/mod.rs` | REWRITE | 同 |
| `src/exact/types.rs` | REWRITE | `payload.rs` |
| `src/exact/error.rs` | REWRITE | 同 |
| `src/exact/eip712.rs` | REWRITE | 同 |
| `src/exact/client.rs` | REWRITE | 同 |
| `src/exact/server.rs` | REWRITE | 同 |
| `src/exact/verify.rs` | REWRITE | `facilitator/verify.rs` |
| `src/exact/facilitator/mod.rs` | REWRITE | 同 |
| `src/exact/facilitator/config.rs` | REWRITE | 同 |
| `src/exact/facilitator/transport.rs` | REWRITE | 同 |

无 `settle.rs`。无 `facilitator/tests.rs`。

### Algorand extras

| Old | Action | New |
| --- | --- | --- |
| `src/chain/codec.rs` | SPLIT | `codec.rs` + `codec/group.rs`（God 814） |
| `src/chain/signer.rs` | REWRITE | 同 |
| `src/chain/rpc.rs` | REWRITE | 单文件（不预建 `rpc/algod.rs`） |

### TVM extras（23 HEAD `.rs`）

| Old | Action | New |
| --- | --- | --- |
| `src/lib.rs` | SPLIT | `lib.rs`；DEFAULT_* → `chain/defaults.rs` |
| `src/networks.rs` | REWRITE | 同 |
| `src/provider.rs` | DROP | merge 进 `chain/provider.rs` + `provider/{toncenter,tonapi}.rs` |
| `src/trace.rs` | DROP | |
| `src/chain/mod.rs` | REWRITE | 同 |
| `src/chain/types.rs` | REWRITE | `chain/account.rs` |
| `src/chain/provider.rs` | REWRITE | 同 + `provider/{toncenter,tonapi}.rs` |
| `src/chain/rpc.rs` | REWRITE | 同 |
| `src/codecs/mod.rs` | REWRITE | `chain/codec/mod.rs` |
| `src/codecs/common.rs` | REWRITE | `chain/codec/cell.rs` |
| `src/codecs/w5.rs` | REWRITE | `chain/codec/w5.rs` |
| `src/codecs/highload_v3.rs` | REWRITE | `chain/codec/highload_v3.rs` |
| `src/codecs/jetton.rs` | REWRITE | `chain/codec/jetton.rs` |
| `src/exact/mod.rs` | REWRITE | 同 |
| `src/exact/types.rs` | REWRITE | `payload.rs` |
| `src/exact/error.rs` | REWRITE | 同 |
| `src/exact/client.rs` | REWRITE | 同 |
| `src/exact/server.rs` | REWRITE | 同 |
| `src/exact/settlement_batcher.rs` | REWRITE | `facilitator/batcher.rs` |
| `src/exact/facilitator/mod.rs` | REWRITE | 同 |
| `src/exact/facilitator/verify.rs` | REWRITE | 同 |
| `src/exact/facilitator/settle.rs` | REWRITE | 同 |
| `src/exact/facilitator/tests.rs` | REWRITE | `tests/exact.rs` |

### 其它点名

| Old | Action | New |
| --- | --- | --- |
| hedera `src/chain/tx.rs` | REWRITE | 同 |
| keeta `src/exact/facilitator/queue.rs` | REWRITE | 同 |
| stellar `src/chain/signer.rs` | REWRITE | 同 |
| stellar `src/chain/xdr.rs` | REWRITE | 单文件（不预建 `xdr/ops.rs`） |
| tron `src/chain/contracts.rs` | REWRITE | 同 |
| tron `src/exact/facilitator/signature.rs` | REWRITE | 同 |
| xrpl `src/chain/codec.rs` | REWRITE | 同 |

夹具 19 个：根树已点名，REWRITE/拷到 `tests/fixtures/`（WT 已有）。

| Old | New |
| --- | --- |
| `r402-core/tests/fixtures/spec_v2/payment_payload_eip3009.json` | `tests/fixtures/spec_v2/payment_payload_eip3009.json` |
| `r402-core/tests/fixtures/spec_v2/payment_required.json` | `tests/fixtures/spec_v2/payment_required.json` |
| `r402-core/tests/fixtures/spec_v2/settle_response_failure.json` | `tests/fixtures/spec_v2/settle_response_failure.json` |
| `r402-core/tests/fixtures/spec_v2/settle_response_success.json` | `tests/fixtures/spec_v2/settle_response_success.json` |
| `r402-core/tests/fixtures/spec_v2/supported_response.json` | `tests/fixtures/spec_v2/supported_response.json` |
| `r402-core/tests/fixtures/spec_v2/verify_response_invalid.json` | `tests/fixtures/spec_v2/verify_response_invalid.json` |
| `r402-core/tests/fixtures/spec_v2/verify_response_valid.json` | `tests/fixtures/spec_v2/verify_response_valid.json` |
| `r402-core/tests/wire_proptest.proptest-regressions` | `tests/fixtures/payment_proptest.proptest-regressions` |
| `r402-algorand/src/exact/fixtures/spec_payment_group.json` | `tests/fixtures/algorand/spec_payment_group.json` |
| `r402-aptos/src/exact/fixtures/extra.json` | `tests/fixtures/aptos/extra.json` |
| `r402-aptos/src/exact/fixtures/ts_encode_aptos_payload.json` | `tests/fixtures/aptos/ts_encode_aptos_payload.json` |
| `r402-hedera/src/exact/fixtures/extra.json` | `tests/fixtures/hedera/extra.json` |
| `r402-keeta/src/exact/fixtures/ts_block.b64` | `tests/fixtures/keeta/ts_block.b64` |
| `r402-keeta/src/exact/fixtures/ts_payment_payload.json` | `tests/fixtures/keeta/ts_payment_payload.json` |
| `r402-near/src/exact/fixtures/ts_signed_delegate.json` | `tests/fixtures/near/ts_signed_delegate.json` |
| `r402-stellar/src/exact/fixtures/extra_sponsored.json` | `tests/fixtures/stellar/extra_sponsored.json` |
| `r402-stellar/src/exact/fixtures/ts_transaction.b64` | `tests/fixtures/stellar/ts_transaction.b64` |
| `r402-tvm/src/exact/fixtures/extra_sponsored.json` | `tests/fixtures/tvm/extra_sponsored.json` |
| `r402-xrpl/src/exact/fixtures/ts_payment_requirements.json` | `tests/fixtures/xrpl/ts_payment_requirements.json` |

---

## DROP（结构）

`r402-core` crate；`Sealed`；registry 三件套；`ChainRegistry`；`casper/hex.rs`；tvm 根 `provider.rs`/`trace.rs`/`codecs/` 名；`solana/upto/shared.rs`；`payment_channels/` 名；各链 `facilitator/tests.rs` 袋子；V1 头字符串；假 `facilitator` feature；localhost/Host 默认；`Paygate*` 类型；`X402Route`；`into_layer`。**不要** `git rm vendor/`（HEAD 无此目录）。

**不要 DROP：** Tron、Casper、两条 upto、EVM auth-capture/batch-settlement、三 mode、Permit2→412。
