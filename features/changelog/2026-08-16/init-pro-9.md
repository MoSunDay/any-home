# init-pro-9：健壮性 + 工程卫生收口（TODO-51..55）

## 背景

在**不动任何对外语义契约**（`MemoryDb`/`MagicRegistry` 公共签名、`fs.db.*` 直连、
魔法路径行为）的前提下做三件卫生事。取证基线：`db/mod.rs` 663 行（唯一逼近 800
迭代门的文件）；src 全树 15 处非测试 `unwrap()`（realfs 5 处锁、magic 9 处锁、
db `count()` 1 处 Option）；`dispatcher/{routing,dbops,readdir}.rs` 三个子模块
零直连测试覆盖。两条 init-pro-8 红线本轮不触碰：每个 WRITE 写穿 DB + fsync、
每个 WRITE 当次全文 embed 一次（写路径逻辑零改动，纯结构/健壮性）。

## 变更

1. **TODO-51 dispatcher 子模块直连单测（行为钉住 + 补缺口）**：
   - `routing.rs`（5 用例）：`dir_category` / `magic_file_in` / `agent_id_of_dir` /
     `magic_parent_of` 纯函数，含 search_k 目录 k 盐化与魔法父目录链。
   - `dbops.rs`（5 用例）：`run_index` 写穿并返回字节尺寸、`run_search` 精确匹配
     top-1 与 agent 隔离、`run_where` FTS / `key:` 前缀 / 空结果 / 畸形 FTS error 行。
   - `readdir.rs`（2 用例）：非魔法目录 fallthrough `Ok(None)`、search_k 不可枚举。
2. **TODO-52 `db/mod.rs` 预防性拆分（行为中性，独立 commit 可二分）**：
   663 → 172 行。新增私有子模块承载 inherent `impl MemoryDb` 三簇：
   `schema.rs`（273：建表/触发器/迁移/dim 簿记，`pub(super)` 保持 crate 内私有）、
   `crud.rs`（126）、`query.rs`（138：KNN + FTS + `classify_fts_error`）。
   `mod.rs` 保留 struct/open/durability 配置 + `WhereOutcome`/`vec_to_json`/`RowGet`
   + 再导出。行为中性证明：行多重集 diff（归一化 use/mod/`pub(super)`/模块注释后）
   仅剩 3 对 impl 块界定符，其余逐字搬运。公共路径（`db::vec_to_json`、
   `db::WhereOutcome`、`lib.rs` 再导出、5 个集成测试 `fs.db.*` 直连）零破坏。
3. **TODO-53 unwrap→可恢复（Result 返回侧）**：realfs 5 处 `.lock().unwrap()` →
   `.lock().map_err(|_| NFS3ERR_IO)?`（锁毒降级为单次 RPC IO 错误，沿用文件内既有
   IO 映射惯例；守卫内容仅 id/path 簿记，恢复续用严格优于整 server 崩溃）；
   `db/schema.rs` `count()` 的 Option unwrap → `ok_or_else(anyhow!)`
   （`SELECT count(*)` 恒一行，空行属真实异常）。
4. **TODO-54 unwrap→可恢复（签名保持侧）**：`magic.rs` 9 处锁 unwrap → 自由函数
   `guard<T>(&Mutex<T>) -> MutexGuard`（`unwrap_or_else(PoisonError::into_inner)`），
   **公共签名零改动**（NFS 面契约红线，不可返回 Result）。权衡已文档化：守卫临界区
   均为纯 Vec/HashMap 簿记（无 unwrap/await/IO panic 源），染毒实际不可达；
   `into_inner()` 恢复严格优于崩溃。
5. **TODO-55 文档三件套 + 计数对账**：本文件 + README 双语计数/结构 +
   `plans/init-pro-9/`（index 决策表 D1–D4 + TODO-51..55）。

## 关键决策（不做什么 / 为什么）

- **不触碰写路径与语义契约**：本轮纯结构/健壮性；两条 init-pro-8 红线由既有测试
  （write_amplification、magic_write_semantics、durability）继续看护。
- **拆分是预防性而非抢救**：663<800 未越线；按方法聚类（schema/迁移、CRUD、查询）
  三簇拆分，模式完全复用 init-pro-8 D1（inherent impl 可跨文件，trait impl 不可；
  `RowGet` trait+impl 留在 mod.rs 同文件，规避 E0119 类风险）。
- **magic 锁毒恢复选 `into_inner()` 而非改签名**：公共签名是契约；锁内无 panic 源，
  理论上的 `BufferTable.total_bytes` 不一致实际不可达。
- **readdir 直连只做最小用例**：`magic_readdir` 机制较重且已被集成测试间接覆盖，
  仅钉 fallthrough 与 search_k 不可枚举两个结构性行为。

## Impact Surface

- `crates/anyhome/src/db/{mod,schema,crud,query}.rs`、`backend/realfs.rs`、
  `magic.rs`、`dispatcher/{routing,dbops,readdir}.rs`（仅新增内联 `#[cfg(test)]`）。
- 对外 API 与行为零变化：集成测试全部原样通过。

## 验收（当次实跑）

- `cargo test --workspace` = **123 passed**（109 基线 + 14 新增：dispatcher 直连 12 +
  锁毒恢复 2）；`--features embed-fastembed` = **127 passed**；零回归。
- `cargo clippy --workspace --all-targets -- -D warnings` 双 feature 零警告。
- `scripts/e2e.sh` exit 0（skip 路径）。
- 行数（最终态）：`db/mod.rs` 172、`db/schema.rs` 273、`db/crud.rs` 126、
  `db/query.rs` 138（新文件均 <400）；`magic.rs` 543、`backend/realfs.rs` 436、
  `dispatcher.rs` 459、`dispatcher/{dbops,routing,readdir}.rs` 237/224/206
  （迭代文件均 <800）。
- 非测试 `unwrap()` src 全树清零；无新增 `#[ignore]` / 删测试 / 弱断言 / 调试输出。
