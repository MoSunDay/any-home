# TODO-48 · 语义保持削减：消除 fattr 二次回读

**状态**：done（spike 收缩：clone 部分被 D3 前提推翻，见下）

## 变更
- `src/dispatcher/dbops.rs::run_index`：返回值 `()` → `u64`（`text.len()`，即 upsert 刚落
  盘的字节数），调用方据此构造 fattr。
- `src/dispatcher.rs::write`：不再调 `index_attr`（其内部 `index_name_for` + `get_text`
  整文回读），改为 `magic_attr_sized(id, size)`。getattr/setattr 保留 `index_attr`
  （无写入发生，尺寸只能问 DB）。
- `tests/write_amplification.rs::hot_write_path_db_read_shape`：断言从 m+1 收紧为 **1**
  （仅首写 seed 探测；热路径每 WRITE 零 `get_text`）。
- `tests/magic_write_semantics.rs`：+1 用例 `write_fattr_reports_this_writes_composed_size`
  （WRITE 返回 post-write 属性 = 本次写入的组合尺寸；getattr 与之吻合）。

## 语义说明（并发同 key 写）
旧路径 fattr 尺寸取自回读（upsert 后另一次查询），并发同 key 双写时可能读到**另一写者**
的长度；新路径报告**本次写入**的组合长度——即 NFS WRITE 契约的 post-write 属性语义，
单写者行为逐字节一致（既有 real-byte-size 用例原样通过）。

## D3：为什么整缓冲 clone 不消（收缩理由）
embed 是 async（`.await`）且 `NFSFileSystem` 的 boxed future 必须 `Send`；std
`MutexGuard` 非 `Send` → embed 输入必须在锁外存活 = 至少一次 O(N) 物化。去 clone 的其余
路径（take/restore 归还式）会改变并发同 id 写的组合语义（另一写者可能基于空缓冲组合，
丢失先写者字节）→ 违反「语义保持」红线。故保留单次 clone 为下界，仅消第二次 DB 回读。

## 验收
- `cargo test --workspace` = **105 passed**（104 + 1 新用例），durability /
  magic_write_semantics 既有断言原样通过；fastembed = 109。
- `cargo clippy --workspace --all-targets -- -D warnings` ✓ 零警告。
