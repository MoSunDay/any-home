# TODO-51 · dispatcher 子模块直连单测（行为钉住 + 补缺口）

**状态**：done（commit ccafc0c）

## 变更
- `src/dispatcher/routing.rs` +129（95→224）：内联 `#[cfg(test)]` 5 用例——
  `dir_category`（全局/agent 作用域/search_k/非魔法目录四分支）、`magic_file_in`
  （scoped id + entry + search_k k 盐化无碰撞）、`agent_id_of_dir`（全局/已注册/
  未注册回退）、`magic_parent_of`（全局四目录/agent 容器/agent 子目录/全局与
  agent search_k 的父链）。
- `src/dispatcher/dbops.rs` +97（140→237）：5 用例——`run_index` 写穿且返回字节
  尺寸、`run_search` 精确匹配 top-1、`run_search` agent 隔离、`run_where` FTS/
  `key:` 前缀/无命中空结果、`run_where` 畸形 FTS 渲染 `# error:` 行。
- `src/dispatcher/readdir.rs` +54（152→206）：2 用例——非魔法目录（根 + 真实
  子目录）fallthrough `Ok(None)`、search_k 目录被处理但不可枚举。
- 惯例：`:memory:` DB（dim 16）+ `DummyEmbedder` + `tempdir().keep()`，与集成
  测试 `make()` 一致；测试位于子模块内，天然可达 `pub(super)` 方法与私有字段。

## 验证
- `cargo test --workspace`：109 → **121 passed**，零回归。
- `--features embed-fastembed`：113 → **125 passed**。
- clippy 零警告。拆分前行为钉住基线成立。
