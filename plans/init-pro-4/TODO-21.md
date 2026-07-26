# TODO-21 DB schema 迁移

## 目标
为 `memory` 表引入 `agent_id` 维度，建立 `UNIQUE(agent_id, key)` 复合唯一约束，消除多 agent 同名 key 碰撞；旧库自动迁移到 v1。

## 核心实现
- `memory` 表新增列 `agent_id TEXT NOT NULL DEFAULT ''`；主键/唯一约束由 `key` 改为 `UNIQUE(agent_id, key)`。
- `meta` 表新增 `schema_version` 行记录当前版本（v0→v1）。
- 启动检测版本：v0 → 建临时新表（带复合唯一）→ `INSERT ... SELECT` 拷数据（旧记录 `agent_id` 取 default `""`）→ DROP 旧表 → RENAME；v1 直接复用。
- 迁移在单连接内完成，保证原子性。

## 验收手段
`tests/agent_namespace.rs`：`migration_carries_legacy_into_default_agent`（v0 旧记忆迁移后归属 default agent，可经全局路径读回）；`composite_unique_allows_same_name_across_agents`。

## 状态
done

## 证据
迁移后 `SELECT DISTINCT agent_id` 包含 `""`；既有 key 经全局 `/index` 路径读回内容一致。

## 依赖
—
