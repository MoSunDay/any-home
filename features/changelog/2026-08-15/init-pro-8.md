# init-pro-8：写路径性能硬化（TODO-46..50）

## 背景

两条写路径契约（**本迭代不触碰**）：① 每个 WRITE RPC 写穿 DB（upsert + commit fsync）；
② 每个 WRITE RPC 对当次全文做一次 embed。TODO-46 spike 以计数 embedder + `get_text`
计数器量化基线（32 KiB × 8 chunk）：

- embed 恰 8 次（每 WRITE 一次，全文输入，总字节 = C·m(m+1)/2）——契约形状确认，渐近项
  O(N²/C) 受保护，只清常数项。
- **热路径每 WRITE 多一次 `get_text` 整文回读**（fattr 尺寸用），首写另 +1 次 seed 探测。
- 整缓冲 clone 每 WRITE 一次（O(N) memcpy ×1，DB 回读另含一次 O(N) 拷出）。

## 变更

1. **TODO-47 dispatcher 拆分（工程门，行为中性）**：`dispatcher.rs` 760 → 455 行，保留
   结构体/鉴权辅助/完整 trait impl；新增 `dispatcher/{dbops,routing,readdir}.rs`
   （135/95/152 行）承载 inherent impl。readdir trait 方法薄委托
   （`magic_readdir -> Ok(None)` 落回真实 FS）。E0119 红线遵守：trait impl 不跨文件。
2. **TODO-48 消二次回读**：`run_index` 返回落盘字节数（`text.len()`），`write` 直接构造
   `magic_attr_sized`，不再 `index_attr` 整文回读。WRITE 返回 post-write 属性（NFS
   契约语义：本次写入的组合尺寸；单写者行为逐字节不变）。getattr/setattr 仍走
   `index_attr`（彼处无写入，尺寸只能问 DB）。
3. **TODO-49 缓冲 LRU + 字节上限**：淘汰从「HashMap 迭代序任意」→「最久未写优先」
   （std-only 逻辑时钟，与 map 同锁）；新增 8 MiB 总驻留字节硬上限，单缓冲自身超限时
   组合后丢弃（DB 权威，下次 re-seed 透明）。
4. **TODO-46 观测设施**：`MemoryDb::text_read_count()`（`Arc<AtomicU64>`，clone 共享）+
   `tests/write_amplification.rs` 计数 embedder。

## 关键决策（不做什么 / 为什么）

- **不做延迟 embed / close-hook / fsync 松动**：契约红线，两契约是耐久性与检索即时性的
  根基（init-pro-6 / init-pro-2 语义）。
- **不去整缓冲 clone**（收缩声明）：embed 为 async 且 trait future 需 `Send`，std
  `MutexGuard` 非 `Send` → embed 输入必须锁外存活 = 至少一次 O(N) 物化；take/restore
  归还式会改变并发同 id 写的组合语义。保留单次 clone 为正确性下界。
- **不做 >8 MiB 大文件的缓冲驻留**：超限即弃（下次 re-seed），以每 chunk 一次 DB 读的
  代价换硬性内存边界——内存安全 > 稀有路径性能。

## Impact Surface（实际组件面）

- `crates/anyhome/src/dispatcher.rs`（-305 行）+ 新 `dispatcher/{dbops,routing,readdir}.rs`
- `crates/anyhome/src/magic.rs`（356 → 490 行：`MagicRegistry.buffers` 类型换
  `BufferTable`，`write_into`/`seed_buffer`/`forget` 内部实现重写，公共 API 签名不变）
- `crates/anyhome/src/db/mod.rs`（+13 行：计数器字段/自增/读取器，纯观测）
- `crates/anyhome/tests/write_amplification.rs`（新，2 用例）；
  `tests/magic_write_semantics.rs`（+1 用例）；`src/magic.rs` 单测 2 → 8
- `README.md` / `README_EN.md` / `plans/init-pro-8/` / `plans/init-pro-6/index.md`（D3 注记）

## 验收

- `cargo build --workspace` ✓（默认 + fastembed）。
- `cargo clippy --workspace --all-targets -- -D warnings` ✓ 零警告（两种组合）。
- `cargo test --workspace` = **109 passed**（102 既有零回归 + 2 spike + 1 fattr + 4 LRU）；
  `--features embed-fastembed` = **113 passed**。
- `tests/durability.rs` / `tests/magic_write_semantics.rs` 既有断言**原样通过**。
- `scripts/e2e.sh` ✓（skip 路径构建 + 启动）。
- TODO-47 拆分独立 commit（65cba00），可二分定位。
