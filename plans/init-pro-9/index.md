# anyhome 实现计划 · init-pro-9

- **project**: init-pro-9（健壮性 + 工程卫生收口 robustness + engineering hygiene，语义保持）
- **created**: 2026-08-16
- **status**: done（TODO-51..55 全部完成）

## Goal

在**不动任何对外语义契约**（`MemoryDb`/`MagicRegistry` 公共签名、`fs.db.*` 直连、
魔法路径行为）的前提下：① 预防性拆分 `db/mod.rs`（663 行，唯一逼近 800 迭代门）；
② 消除 src 全树 15 处非测试 `unwrap()` 为可恢复处理；③ 补齐 `dispatcher/` 子模块
直连测试（原零直连覆盖）。

## 契约红线（本迭代不触碰）

1. 每个 WRITE RPC 写穿 DB（upsert + commit fsync）——继承 init-pro-8。
2. 每个 WRITE RPC 对当次全文做一次 embed——继承 init-pro-8。
3. `MagicRegistry`/`MemoryDb` 公共签名与 `fs.db.*` 直连路径不变。
写路径逻辑零改动，本轮纯结构/健壮性；红线由既有测试继续看护。

## 关键技术决策

| 决策 | 结论 | 证据 |
|---|---|---|
| D1 db 拆分模式与粒度 | 预防性（663<800 未越线）；按方法聚类三簇 `db/{schema,crud,query}.rs` 承载 inherent `impl MemoryDb`（复用 init-pro-8 D1：inherent impl 可跨文件，trait impl 不可）；schema 簇 `pub(super)` 保持 crate 内私有；`RowGet` trait+impl 留 mod.rs 同文件；公共项（`WhereOutcome`/`vec_to_json`/再导出）留 mod.rs 保路径 | TODO-52 |
| D2 magic 锁毒策略 | 公共签名是 NFS 面契约不可返回 Result → 自由函数 `guard()` 用 `PoisonError::into_inner()` 恢复续用；守卫临界区纯 Vec/HashMap 簿记无 panic 源，染毒实际不可达，恢复严格优于整 server 崩溃（权衡文档化于 guard 注释与 changelog） | TODO-54 |
| D3 Result 返回侧 unwrap | realfs 5 处锁 unwrap → `NFS3ERR_IO`（沿用文件内既有 IO 映射惯例）；`count()` Option → `anyhow!`（`SELECT count(*)` 恒一行，空行是真实异常） | TODO-53 |
| D4 补测策略 | routing/dbops 纯函数/DB 操作用内联 `#[cfg(test)]` 直连单测（`pub(super)` 需同模块树）；readdir 机制较重仅钉最小结构性行为（fallthrough `Ok(None)` + search_k 不可枚举），不做全量 | TODO-51 |

## 状态总表

| TODO | 标题 | 状态 | 依赖 |
|---|---|---|---|
| TODO-51 | dispatcher 子模块直连单测（行为钉住 + 补缺口） | done | — |
| TODO-52 | db/mod.rs 行为中性拆分（独立 commit，可二分） | done | TODO-51 |
| TODO-53 | unwrap→可恢复（Result 返回侧）：realfs 5 + db count 1 | done | TODO-52 |
| TODO-54 | unwrap→可恢复（签名保持侧）：magic.rs 9 处 guard() | done | TODO-53 |
| TODO-55 | 文档三件套 + 计数对账 | done | TODO-51..54 |

## 全局验收 gate

- TODO-51..55 全 done，每个一个独立 commit。
- `cargo build --workspace` / `cargo clippy --workspace --all-targets -- -D warnings` /
  `cargo test --workspace` 双 feature 全绿。
- `cargo test` = **123 passed**（109 基线 + 12 dispatcher 直连 + 2 锁毒恢复）；
  `--features embed-fastembed` = **127 passed**；既有 109/113 零回归。
- 行为中性证明（TODO-52）：行多重集 diff 仅剩 impl 块界定符；公共路径 grep +
  5 个 `fs.db.*` 直连集成测试原样通过。
- 行数（最终态）：`db/mod.rs` 172、`db/schema.rs` 273、`db/crud.rs` 126、
  `db/query.rs` 138（新文件均 <400）；`magic.rs` 543、`backend/realfs.rs` 436、
  `dispatcher.rs` 459、`dispatcher/{dbops,routing,readdir}.rs` 237/224/206
  （迭代文件均 <800）。
- 非测试 `unwrap()` src 全树清零；无新增 `#[ignore]`/删测试/弱断言/调试输出。
- `scripts/e2e.sh` exit 0（skip 路径）。
