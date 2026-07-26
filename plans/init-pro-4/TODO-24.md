# TODO-24 /agents 虚拟命名空间多层路由

## 目标
在 NFS 路径树中提供 `/agents/<agent_id>/{search,index,where}` 多层虚拟目录，支持 `lookup`/`readdir` 逐层解析。

## 核心实现
- `lookup` / `readdir` 识别 `/agents` → per-agent 目录 → 三魔法端点的多层结构。
- `/agents` 目录枚举 `list_agents()` 返回的 agent_id 列表（动态，来自 DB）。
- per-agent 目录枚举其 `search`/`index`/`where` 三个魔法端点（MagicEntry）。
- 全局根 `/` 同时保留 `/agents` + 旧 `/search`、`/index`、`/where`，路由统一经 MagicEntry。
- 新增 `CAT_AGENT` 系列常量区分目录与魔法端点。

## 验收手段
`tests/agent_namespace.rs`：`agents_dir_lists_existing_agents`；`per_agent_dir_lists_magic_endpoints`；
`lookup_resolves_agents_subpath`。

## 状态
done

## 证据
写入 agent 后 `/agents` readdir 出现该 agent_id；`/agents/<id>/index` 可 lookup 并读写。

## 依赖
TODO-22, TODO-23。
