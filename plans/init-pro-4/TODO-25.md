# TODO-25 Agent-scoped 读写语义 + 向后兼容

## 目标
确立 per-agent 读写语义并保证全局旧路径向后兼容：全局 `/search`、`/index`、`/where` 映射 default agent `agent_id=""`。

## 核心实现
- dispatcher 在解析魔法端点时携带 `agent_id`：全局路径 → `agent_id=""`；`/agents/<id>/...` → 对应 agent_id。
- 读写调用（`get_text`/`upsert`/`exists`/`list_with_size`）按解析到的 agent_id 访问 DB。
- 写入缓冲 per-fileid 天然隔离；agent-scoped fileid 保证不同 agent 并发写入不串扰。
- default agent 与具名 agent 互不可见（namespace 隔离）；init-pro-3 的 offset 拼接、真实大小、ROFS 等语义全部保留。

## 验收手段
`tests/agent_namespace.rs`：`global_paths_map_to_default_agent`（全局路径读写等价 init-pro-3）；
`per_agent_index_isolated` / `per_agent_search_isolated`（具名 agent 与 default 互不可见）。

## 状态
done

## 证据
全局 `/index` 写入与 `/agents/bot1/index` 写入同名 key 互不覆盖；全局路径行为与 init-pro-3 测试一致。

## 依赖
TODO-24。
