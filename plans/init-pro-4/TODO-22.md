# TODO-22 DB 查询按 agent_id 参数化

## 目标
所有 DB 读写查询带上 `agent_id` 维度，实现 per-agent 数据分片；并提供 agent 枚举接口。

## 核心实现
- `get_text(agent_id, key)` / `upsert(agent_id, key, text)` / `exists(agent_id, key)` /
  `list_with_size(agent_id)` / `delete(agent_id, key)`：签名新增 `agent_id: &str`，WHERE 带 `agent_id = ?`。
- 新增 `list_agents()`：`SELECT DISTINCT agent_id FROM memory ORDER BY agent_id`，供 `/agents` 目录枚举。
- 默认参数 `agent_id=""` 对应全局路径，语义等价于 init-pro-3。

## 验收手段
`tests/agent_namespace.rs`：`same_name_key_isolated_across_agents`（两 agent 同名 key 各自独立）；
`list_agents_returns_existing`（枚举结果含已写入 agent）。

## 状态
done

## 证据
两 agent 写同名 key 后各自 `get_text` 返回各自内容，互不影响；`list_agents` 返回两者。

## 依赖
TODO-21。
