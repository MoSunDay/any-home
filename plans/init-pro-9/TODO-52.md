# TODO-52 · db/mod.rs 行为中性拆分（独立 commit，可二分）

**状态**：done（commit 108c14f）

## 变更（单独 commit，可二分定位）
- `crates/anyhome/src/db/mod.rs`：663 → 172 行。保留：模块文档、struct、
  `open`/`configure_durability`/`durability`/`dim`/`text_read_count`、
  `WhereOutcome`/`vec_to_json`/`RowGet`（trait+impl 同文件）、embedder 再导出、
  私有 `mod crud; mod query; mod schema;` 声明。
- `src/db/schema.rs`（新，269→273 行）：`init_schema`/`create_fts_triggers`/
  `migrate_schema`/`schema_version`/`set_schema_version`/`has_column`/
  `reconcile_dim`/`ensure_fts_populated`/`count` + `SCHEMA_VERSION` 常量；
  `pub(super)` 保持 crate 内私有（mod.rs 的 `open` 经 db 模块可见性调用）。
- `src/db/crud.rs`（新，126 行）：`upsert`/`delete`/`list_agents`/`list_keys`/
  `list_with_size`/`get_text`/`exists`，pub 签名逐字不变。
- `src/db/query.rs`（新，138 行）：`search`/`where_search`/`key_prefix_search` +
  自由函数 `classify_fts_error`（仅 where_search 使用，保持私有）。
- 模式同 init-pro-8 D1：inherent impl 跨文件拆分（E0119 只限 trait impl）；
  子模块私有 `mod`，不新增任何公共路径。

## 行为中性验证
- 行多重集 diff（归一化 use/mod/`pub(super)`/`//!` 模块注释后）：旧文件与新四文件
  合集的差集仅剩 3 对 `impl MemoryDb {`/`}` 界定符，其余逐字搬运。
- 公共路径 grep 全调用方 + 编译通过：`db::vec_to_json`（db_vector.rs）、
  `db::WhereOutcome`（dispatcher/dbops.rs）、`lib.rs` 再导出
  `db::{embedder, MemoryDb}`、5 个集成测试 `fs.db.*` 直连原样通过。
- `cargo test --workspace` = **121 passed**（与拆分前一致，零回归）；
  `--features embed-fastembed` = 125 passed；clippy 零警告。
