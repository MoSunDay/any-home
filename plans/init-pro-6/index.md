# anyhome 实现计划 · init-pro-6

- **project**: init-pro-6（写入耐久性硬化 + 写缓冲生命周期修复 write durability hardening）
- **created**: 2026-08-04
- **status**: done（TODO-35..39 全部完成）

## Goal

关闭 README「已知限制」中关于「`/index` 写入缓冲在崩溃时丢失数据」的开口项。

**关键前提更正（TODO-35 spike 结论）**：调研 + 实测证明该限制的描述是**错误的**——
`/index` 写路径**早已 write-through**（每次 `WRITE` RPC 立即 `upsert` 到 DB，并不缓冲至 close），
且 bundled libsql（core-only）默认 `synchronous=FULL`，每次已提交写即 fsync。因此**不存在**
「崩溃丢缓冲」的数据丢失问题。

真实缺口（本迭代修复）：

1. **耐久性非显式**：依赖 bundled SQLite 编译期默认，未在代码中声明，升级依赖后可能悄悄改变；
2. **`delete` 日志模式**：写入时阻塞并发读，NFS 多客户端并发检索受影响；
3. **`buffers` HashMap 泄漏**：仅在 `remove` 时清理，长期运行 + 多 key 写入会无限增长内存；
4. **README 描述陈旧/错误**。

## 关键技术决策

| 决策 | 结论 | 证据 |
|---|---|---|
| D1 耐久性策略 | `journal_mode=WAL` + `synchronous=FULL`：并发读不阻塞写，且每个已提交写 fsync（满耐久） | `db/mod.rs::configure_durability` |
| D2 写穿语义 | 维持「每次 WRITE 即 upsert」（已存在），**不**引入 WAL/replay 机制——没有需要 replay 的缓冲 | `dispatcher.rs::write` → `run_index` → `db.upsert` |
| D3 缓冲淘汰 | 上限 256 条目，满时淘汰任意一条；DB 为权威来源，淘汰安全（下次写从 DB re-seed） | `magic.rs::MAX_BUFFERS` + `write_into` |
| D4 PRAGMA 通过 query | `journal_mode=WAL` 返回行，libsql `execute` 拒绝返回行的语句 → 用 `query` | `configure_durability` |

## 状态总表

| ID | 标题 | 状态 | 依赖 |
|----|------|------|------|
| TODO-35 | 现状基线 spike（实测默认 PRAGMA + 写穿语义确认） | done | — |
| TODO-36 | 显式耐久性 PRAGMA（WAL + synchronous=FULL） | done | TODO-35 |
| TODO-37 | 写缓冲生命周期硬化（MAX_BUFFERS 上限 + 安全淘汰） | done | TODO-35 |
| TODO-38 | 耐久性 + 缓冲验证测试 | done | TODO-36, TODO-37 |
| TODO-39 | 文档更新（修正 README 已知限制 + 测试计数） | done | TODO-36..38 |

## 全局验收 gate
- TODO-35..39 全 done。
- `cargo build` / `cargo clippy --workspace --all-targets -- -D warnings` / `cargo test --workspace` 双 feature 全绿。
- `cargo test` = **87 passed**（82 既有 + 3 durability + 2 magic 单测）；`--features embed-fastembed` = **91 passed**。既有用例零回归。
- 行数：新增/迭代文件均在限内（`magic.rs` 306 / `db/mod.rs` 607 / `durability.rs` 47）。
