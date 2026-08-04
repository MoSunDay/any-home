# TODO-35 · 现状基线 spike

**状态**：done

## 目标
实测 bundled libsql（core-only）的默认耐久性设置，并确认 `/index` 写路径的真实数据流，
以判断「崩溃丢缓冲」已知限制是否真实存在。

## 结论（关键前提更正）
- `PRAGMA journal_mode` = **delete**，`PRAGMA synchronous` = **2 (FULL)**：每次已提交写即 fsync。
- `/index` 写路径**早已 write-through**：`dispatcher.rs::write` → `run_index` → `db.upsert`，
  每次 `WRITE` RPC 立即 upsert（无 close/commit handler）。内存缓冲仅为偏移拼接辅助。
- 因此 README「写入数据缓冲在内存中，close 时才 upsert；崩溃时未落盘的缓冲会丢失」**描述错误**——
  不存在该数据丢失问题。

## 真实缺口（驱动后续 TODO）
1. 耐久性依赖编译期默认、未显式声明。
2. `delete` 模式写入时阻塞并发读。
3. `buffers` HashMap 仅在 `remove` 清理，会泄漏。
4. README 描述陈旧。

## 验证方式
临时 spike 测试查询 PRAGMA 默认值（已删除）；最终验证由 TODO-38 的持久化测试覆盖。
