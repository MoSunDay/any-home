# TODO-47 · dispatcher.rs 拆分（工程门，行为中性）

**状态**：done

## 变更（单独 commit，可二分定位）
- `crates/anyhome/src/dispatcher.rs`：760 → 453 行。保留：结构体 + `new`/`with_auth`/
  `current_caller`/`require_agent` + 完整 `NFSFileSystem` trait impl；`readdir` trait 方法
  变为薄委托（`magic_readdir` 返回 `Ok(None)` 时落回真实 FS + 根目录魔法目录追加）。
- `src/dispatcher/dbops.rs`（新，135 行）：`run_search` / `run_index` / `run_where` /
  `index_text` / `index_name_for` / `index_attr`——DB 支撑操作，inherent `impl AnyHomeFs`。
- `src/dispatcher/routing.rs`（新，95 行）：`dir_category` / `magic_file_in` /
  `agent_id_of_dir` / `magic_parent_of`——纯 registry 解析，无 DB、无 await。
- `src/dispatcher/readdir.rs`（新，152 行）：`magic_readdir`——/agents、/agents/<id>、
  /search_k<N>（不可枚举）、可载文件的魔法目录四个分支原样迁入。
- 模式：Rust 2018 `dispatcher.rs` + `dispatcher/` 同名并存；trait impl（E0119 红线）
  不跨文件——子模块全部是 inherent impl；方法 `pub(super)`；`pub real/db/embedder`
  可见性与模块路径不变（5 个集成测试直访 `fs.db` 不受影响）。

## 行为中性验证
- 脚本化 diff 对账：旧/新（归一化 `pub(super)`/`Some()` 包装与 use 行后）代码行
  多重集合完全一致，唯一新增行为 = readdir 委托线。
- `cargo build` ✓ / `cargo clippy --workspace --all-targets -- -D warnings` ✓ 零警告。
- `cargo test --workspace` = **104 passed**（与拆分前完全一致，零回归）；
  `--features embed-fastembed` = 108 passed。
- `scripts/e2e.sh` ✓（skip 路径构建+启动验证）。
