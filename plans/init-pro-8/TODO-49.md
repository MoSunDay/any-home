# TODO-49 · 写缓冲 LRU + 8 MiB 字节上限

**状态**：done

## 变更（均在 `crates/anyhome/src/magic.rs`，356 → 493 行）
- `MAX_BUFFER_BYTES = 8 MiB`（新常量）：**总驻留字节硬上限**；条目上限 256 保持。
- `BufferEntry { data, last_used }` + `BufferTable { map, tick, total_bytes }`：逻辑时钟
  与字节总量与 map 同锁（`buffers: Mutex<BufferTable>`），淘汰决策永远不跨锁。
- `write_into` → `BufferTable::splice`（原位拼接 + 时钟戳 + 总量增量）+ `enforce_caps`：
  淘汰从「HashMap 迭代序任意一条」改为「**最久未写优先**」（O(n) 扫描，n ≤ 256）。
- **硬边界语义**：单个缓冲内容自身超限（如 > 8 MiB 的 key）时，组合完成后**丢弃该缓冲**
  （DB 为权威来源，下次写从 DB re-seed 透明恢复）；write 返回值仍是完整组合文本，
  embed/upsert 不受影响。`seed_buffer` 不自行 enforce——seed 后同 RPC 的 splice 先组合
  再淘汰，保证不出现「seed 被淘汰 → 空缓冲拼接 → 内容损坏」。
- `forget` 同步扣减 `total_bytes`。

## 测试（magic.rs 单测 4 → 8，均零 I/O、<10ms 量级；两个大缓冲用例瞬时 ~20 MiB）
- `lru_keeps_hot_keys_resident`：301 distinct vs 256 cap，45 个受害者全部严格早于 mid-churn
  刷新的热 key（热 key 存活、old0 被淘汰、上限成立）。
- `byte_cap_evicts_lru_first`：3×3 MiB，第 3 个触发 9 MiB > 8 MiB → 淘汰最久未写的 a，
  b/c 保留。
- `single_oversize_buffer_is_dropped_not_hoarded`：单缓冲 > cap → 不驻留但组合文本照常返回。
- `oversize_seed_composes_before_drop`：>cap 的 seed + 追加 → 组合含 seed+新字节后才丢弃。
- 既有 `write_buffers_are_bounded` / `eviction_is_transparent` 原样通过（改 `map.len()`）。

## 验收
- `cargo test --workspace` = **109 passed**（105 + 4 新单测），fastembed = 113。
- `cargo clippy --workspace --all-targets -- -D warnings` ✓ 零警告（entry API 修 map_entry）。
- `scripts/e2e.sh` ✓（skip 路径）。
