# TODO-37 · 写缓冲生命周期硬化

**状态**：done

## 目标
修复 `MagicRegistry::buffers` 的内存泄漏（仅在 `remove` 时清理）。

## 变更
- `magic.rs`：新增 `MAX_BUFFERS = 256`。
- `write_into`：当缓冲数 ≥ MAX_BUFFERS 且 `id` 尚无缓冲时，淘汰任意一条后再插入。
- 淘汰安全：DB 为权威来源（每次 WRITE 已 upsert），下次写到被淘汰 id 会从 DB re-seed。
- NFSv3 无状态、trait 无 `close`/`commit` 钩子，故无法按文件关闭即时回收；上限 + 安全淘汰
  是无依赖、O(1) 的正确解法。

## 单测
`magic::tests::{write_buffers_are_bounded, eviction_is_transparent}` 锁定上限与淘汰后拼接正确性。
