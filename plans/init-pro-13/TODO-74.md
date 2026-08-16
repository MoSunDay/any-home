# TODO-74 · schema v2：ANN 索引退役 + 维度门禁保留

## 产出

- `src/db/schema.rs` 273→291（+18）：`SCHEMA_VERSION` 1→2；迁移末尾无条件
  `DROP INDEX IF EXISTS memories_vec_idx`（新库无此索引、老库级联清影子表，
  一条语句两吃）。init-pro-12 起 `search` 是 agent 分区暴力 KNN，索引零
  查询命中，只剩逐写维护成本。
- `src/db/schema.rs` 新增 `check_emb_dim` + `src/db/crud.rs` 126→129
  （+3）：**计划外但必须**——索引存在时 libsql 会在写路径强制维度校验，
  退役索引后该校验消失；补回显式门禁，维度不匹配仍报
  `vector dimensions are different`。公共签名只增不改。
- `tests/schema_v2.rs` 新文件（195 行，+2 例）：
  - `new_db_is_schema_v2_and_creates_no_vector_index`：新库 version=2 且
    `sqlite_master` 无任何 vec/shadow 对象。
  - `v1_db_reopen_drops_index_and_keeps_data_and_search`：手工伪造 v1 库
    （含真实 `libsql_vector_idx`）→ 用 `MemoryDb` 重开 → 索引/影子表清零、
    数据与 search 行为完好。
- `tests/db_persistence.rs` 180→204（+24）：版本断言 1→2 同步。
- `tests/db_vector.rs`：注释更新（KNN 已是暴力扫描，非索引）。

## 实测

- 单测 +2 全绿；部署库实弹迁移在 TODO-76 验证：`meta.schema_version=2`，
  `memories_vec_idx`/`memories_vec_idx_shadow`/`memories_vec_idx_shadow_idx`
  全部消失，`libsql_vector_meta_shadow` 保留为**空表**（对应行已级联删除；
  全局注册表本身是惰性基础设施，无写成本）。
- 维度门禁回归：`db_vector` 既有维度不匹配用例继续钉死报错文案。
