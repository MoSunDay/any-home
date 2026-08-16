# TODO-54 · unwrap→可恢复（签名保持侧）：magic.rs 9 处 guard()

**状态**：done（commit 691bedd）

## 变更
- `src/magic.rs`：新增自由函数
  `fn guard<T>(lock: &Mutex<T>) -> MutexGuard<'_, T>`
  （`lock.lock().unwrap_or_else(PoisonError::into_inner)`），替换 `MagicRegistry`
  全部 9 处 `.lock().unwrap()`（entries：register/get_entry/get/forget/
  list_for_cat；buffers：forget/has_buffer/seed_buffer/write_into）。
- **公共签名零改动**（NFS 面契约红线，不可返回 Result）。
- 权衡（文档化于 guard 注释 + changelog）：染毒意味着锁内曾 panic，理论上
  `BufferTable.total_bytes` 可能不一致；但守卫临界区均为纯 Vec/HashMap 簿记
  （无 unwrap/await/IO panic 源），染毒实际不可达；若真发生，恢复续用严格优于
  整 server 崩溃。
- 新增锁毒恢复测试（magic.rs 内联）：`Arc<MagicRegistry>` 分别染毒 buffers 与
  entries 两把锁，断言 `write_into`/`has_buffer`/`register`/`get`/`get_entry`/
  `list_for_cat`/`forget` 全部照常工作、不 panic。

## 验证
- `cargo test --workspace`：122 → **123 passed**；`--features embed-fastembed`：
  126 → **127 passed**；clippy 零警告。
- src 全树非测试 `unwrap()` 清零（脚本扫描确认）。
