# TODO-38 · 耐久性 + 缓冲验证测试

**状态**：done

## 变更
- `tests/durability.rs`（新，3 用例）：
  - `file_db_uses_wal_and_full_sync`：文件 DB = WAL + synchronous=FULL。
  - `memory_db_stays_ephemeral_without_error`：`:memory:` 保持 `"memory"`、不报错。
  - `write_through_survives_reopen`：写穿 → drop → reopen → 数据存活。
- `magic.rs` 单测（2 用例）：缓冲上限、淘汰后拼接透明。
- 全部非 `#[ignore]`。

## 回归保护
WAL 改动影响**所有** `MemoryDb::open`，故既有 82 用例（含 db_persistence / vector / fts）
全量回归通过即证明 WAL 与既有功能兼容（vector 索引 + FTS5 + 迁移均在 WAL 下正常）。
