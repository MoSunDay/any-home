# init-pro-6：写入耐久性硬化 + 写缓冲生命周期修复（TODO-35..39）

## 背景

README「已知限制」列出「`/index` 写入缓冲在崩溃时丢失数据」，计划候选 A（写入崩溃安全）
据此立项。但 **TODO-35 spike 实测推翻了该前提**：

- `/index` 写路径**早已 write-through**——每次 `WRITE` RPC 立即 `upsert` 到 DB（无 close/commit handler）；
- bundled libsql（core-only）默认 `synchronous=FULL`，每次已提交写即 fsync。

即「崩溃丢缓冲」的数据丢失问题**并不存在**，README 描述是错误的。本迭代改为修复 spike
揭示的**真实缺口**，并更正文档。

## 真实缺口与修复

1. **耐久性非显式** → `configure_durability()` 显式设 `journal_mode=WAL` + `synchronous=FULL`。
   - WAL：并发 NFS 读不阻塞写；
   - FULL：每个已提交写 fsync，断电不丢（与「记忆服务不应丢已确认写」一致）；
   - `:memory:` 不受影响（WAL 不适用，返回 `"memory"` 不报错）。
2. **写缓冲泄漏** → `MAX_BUFFERS = 256` 上限 + 满时安全淘汰（DB 为权威来源，淘汰后下次写从 DB re-seed）。
   NFSv3 无状态、trait 无 close 钩子，无法按关闭即时回收，故采用上限 + 淘汰。
3. **README 陈旧/错误** → 已知限制改为准确的 write-through 耐久性说明；测试计数更新。

## 关键决策（不做什么）

- **不引入 WAL/写穿 replay 机制**：没有需要 replay 的缓冲（数据早已 write-through）。
  避免为一个不存在的问题堆叠机制（Reliability Beats Coverage / 最小必要更新）。

## 变更文件

- `crates/anyhome/src/db/mod.rs`：`configure_durability()` + `durability()`，`open()` 首发调用。
- `crates/anyhome/src/magic.rs`：`MAX_BUFFERS` + `write_into` 淘汰逻辑 + 2 单测。
- `crates/anyhome/tests/durability.rs`（新，3 用例）。
- `README.md` / `README_EN.md`：已知限制更正、能力说明、测试计数。

## 验收

- `cargo build --workspace` ✓（默认 + `--features embed-fastembed`）。
- `cargo clippy --workspace --all-targets -- -D warnings` ✓ 零警告。
- `cargo test --workspace` = **87 passed**（82 既有 + 3 durability + 2 magic 单测）；`--features embed-fastembed` = **91 passed**。
  既有用例**零回归**（WAL 改动影响所有 `open`，全量回归通过证明兼容性）。
- 行数：`magic.rs` 306 / `db/mod.rs` 607（迭代中，<800）/ `durability.rs` 47（<400）。

## 风险与权衡

- **WAL 文件**：文件型 DB 旁会生成 `-wal` / `-shm` 文件；SQLite 默认自动 checkpoint（1000 页），
  无需手动管理。
- **淘汰为「任意一条」**：非 LRU，可能偶发淘汰热点缓冲导致一次 re-seed 读；上限小、re-seed 为
  索引主键读，代价可忽略，且保证正确性。如未来写并发成为瓶颈可升级为 LRU（需引依赖）。

## Related Docs
- [plans/init-pro-6/index.md](../../../plans/init-pro-6/index.md)
- [既有 changelog：init-pro-5](../2026-07-27/init-pro-5.md)
