# TODO-26 集成测试

## 目标
为多 agent 命名空间隔离补端到端集成测试，覆盖 schema 迁移、复合唯一、隔离、向后兼容、fileid 编码。

## 核心实现
- 新增 `tests/agent_namespace.rs`（12 个用例）：
  - schema 迁移：`migration_carries_legacy_into_default_agent`。
  - 复合唯一：`composite_unique_allows_same_name_across_agents` / `same_name_key_isolated_across_agents`。
  - agent 枚举：`list_agents_returns_existing` / `agents_dir_lists_existing_agents`。
  - 路由：`per_agent_dir_lists_magic_endpoints` / `lookup_resolves_agents_subpath`。
  - 隔离：`per_agent_index_isolated` / `per_agent_search_isolated`。
  - 向后兼容：`global_paths_map_to_default_agent`。
  - fileid 编码：`scoped_fileid_distinct_per_agent` / `scoped_name_separator_unambiguous`。
  - default 归属：`default_agent_scoped_under_empty_id`。

## 验收手段
`cargo test --workspace` = **62 passed**（默认）；`--features embed-fastembed` = **66 passed**；clippy 零警告。

## 状态
done

## 证据
12 个 agent_namespace 用例全绿；既有 magic 测试无回归。

## 依赖
TODO-21..25。
