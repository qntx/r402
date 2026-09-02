# r402 2026 layout contract

本目录是施工合同。实现以这里为准，不以 git 历史里的旧路径为准，不以已作废的 408 / 「Approved for construction」树为准。

**Status：** [DESIGN.md](DESIGN.md) 是 **construction contract**。384 个可枚举 `.rs`（**不是** CI 门）。HTTP **shape B**。三 SettlementMode KEEP。upto KEEP（EVM Permit2 + Solana escrow）。`r402-contract` unpublished。

**规范钉：** `/Users/xu/Desktop/x/x402` @ `0344bdf54bf962e758480e76088eb2dc3209a169`（`@x402/*` **2.24.0**）。地面真相 HEAD `fb0e1cdfa4990a15cf6a78f7c5ee7e5c646f9b6a` / tag `pre-wipe-0.17.1`。HEAD **无 `vendor/`**；不要 `git rm vendor/`；禁止再加入。

**验收：** 禁名不在树 + `r402-contract` JSON 绿 + 无 God 回归 + TARGET 强制路径都存在。**不是** `wc -l == 384`。

| 文件 | 内容 |
| --- | --- |
| [DESIGN.md](DESIGN.md) | 施工合同全文：审计、Key Decisions、shape B、PR Plan |
| [CURRENT_TREE.md](CURRENT_TREE.md) | HEAD `fb0e1cd` 每个 `.rs`：路径、LOC、职责、God/junk。295 / 86 256 |
| [TARGET_TREE.md](TARGET_TREE.md) | 修正目标树；384 个强制 `.rs`；无 `X402Route` / `into_layer`；无装饰单孩子 |
| [FILE_MAP.md](FILE_MAP.md) | 旧路径 → REWRITE/SPLIT/DROP。named row ∪ named glob ∪ DROP 覆盖全部 295 HEAD `.rs` |
| [RULES.md](RULES.md) | 闭合规则 1–46。MSRV 1.95（aptos-sdk）。RULE 9 = 填入 PR 拆 God |
| [NAMES.md](NAMES.md) | 命名法 + 文件/文件夹计数（384） |

过时文档（横幅，未删）：[`docs/gap-checklist.md`](../gap-checklist.md)、[`docs/chain-parity-design.md`](../chain-parity-design.md)、[`docs/parity-closure-design.md`](../parity-closure-design.md)、[`docs/structure-aesthetics-redesign.md`](../structure-aesthetics-redesign.md)。
