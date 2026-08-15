# TODO-46 · spike：写放大基线量化

**状态**：done

## 变更
- `crates/anyhome/tests/write_amplification.rs`（新，2 用例）：
  `CountingEmbedder` 包装（embed 次数/字节，行为不变委托）+ 32 KiB × 8 chunk 写入：
  - `every_write_rpc_embeds_full_text_once_and_upserts_write_through`：断言 embed 次数
    = ⌈N/C⌉ = 8；每次 WRITE 返回即 durable（get_text == 已写前缀，一 WRITE 一 upsert）；
    总 embed 字节 = C·m(m+1)/2（受契约保护的 O(N²/C) 形状）。
  - `hot_write_path_db_read_shape`：get_text 计数形状断言（基线 m+1，见下）。
- `crates/anyhome/src/db/mod.rs`：`MemoryDb` 增加 `text_reads: Arc<AtomicU64>` 计数器
  （get_text 自增、`text_read_count()` 读取、clone 共享计数），纯观测无行为变化。

## 基线结论（写入 plans/init-pro-8/index.md 决策 D5）
- embed：每 WRITE 恰 1 次、每次全文（契约形状确认，渐近项受保护）。
- **热路径每 WRITE 多 1 次 get_text 回读**（fattr 尺寸用，`index_attr`），首写另 +1 次
  seed 探测 → m 次 WRITE = m+1 次 get_text。
- 缓冲整量 clone 每 WRITE 1 次（O(N) memcpy）；DB 回读另含一次 O(N) 文本拷出。
- 结论：回读与 clone 是语义无关成本，48 值得做；但 clone 受 Send/async 下界约束
  （D3），实际收缩为「消二次回读」。回读在小文本下无感、在 256 KiB 级文本下是
  每 chunk 一次的 SQLite 全文取回，量级与 embed 输入同阶——可清零且零风险。

## 验收
- `cargo test --test write_amplification` → 2 passed。
- `cargo test --workspace` → 104 passed（102 基线 + 2），零回归。
