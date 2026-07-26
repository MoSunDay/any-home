# init-pro-4：多 Agent 命名空间隔离（TODO-21..27）

## 背景

README L187 将**多 agent 命名空间隔离**列为当前最大的结构性缺口，并明确三点未实现项：
DB 无 `agent_id` 列、魔法路径全局、无鉴权。深入审视代码确认缺陷为：

- **DB 无 `agent_id` 维度**：`memory(key TEXT UNIQUE, text TEXT)` 全局单表，多 agent 同名 key 直接碰撞。
- **魔法路径是全局单例**：`/search`、`/index`、`/where` 各只有一个实例，无 per-agent 视图。
- **fileid 哈希无 agent 维度**：`FNV-1a(name)` 仅按 name 计算，不同 agent 的同名记忆生成**相同 fileid**，句柄跨 agent 串扰。

本迭代交付 **命名空间隔离（不含鉴权）**：在 `/agents/<agent_id>/...` 下提供 per-agent 的 search/index/where 三件套，
DB 与 fileid 编码全部按 agent 分片，旧的全局 `/search`、`/index`、`/where` 路径保持向后兼容（映射到 default agent `agent_id=""`）。

## 变更

### TODO-21 DB schema 迁移（`db/mod.rs`）
- `memory` 表新增 `agent_id TEXT NOT NULL DEFAULT ''` 列；唯一约束由 `UNIQUE(key)` 改为
  `UNIQUE(agent_id, key)`（复合唯一），消除跨 agent 同名碰撞。
- `meta` 表新增 `schema_version` 行；启动时检测版本，v0→v1 执行**表重建迁移**（建新表 → 拷数据 → 替换），
  旧库中既有记忆自动归入 default agent `agent_id=""`。

### TODO-22 DB 查询按 agent_id 参数化（`db/mod.rs`）
- 所有读写查询（`get_text`/`upsert`/`exists`/`list_with_size`/`delete`）签名新增 `agent_id: &str` 参数，
  WHERE 子句带 `agent_id = ?`。
- 新增 `list_agents()`：`SELECT DISTINCT agent_id FROM memory`，供 `/agents` 目录枚举已存在的 agent。

### TODO-23 Agent-scoped fileid 编码（`magic.rs` + `dispatcher.rs`）
- 引入 `scoped_name = "{agent_id}\0{name}"`（`\0` 分隔，合法 name 不含 `\0` 故无歧义），
  fileid 仍由确定性 `FNV-1a(scoped_name)` 计算 → 同名记忆在不同 agent 下 fileid 不同，彻底消除句柄串扰。
- 新增 `MagicEntry` 结构（携带 `agent_id` + `name` + `cat`）；新增 `CAT_AGENT` 系列常量与 `/agents` 目录 category。

### TODO-24 /agents 虚拟命名空间多层路由（`dispatcher.rs`）
- `lookup`/`readdir` 支持 `/agents/<agent_id>/{search,index,where}` 多层路径解析。
- `/agents` 目录枚举 `list_agents()` 结果；per-agent 目录枚举其 search/index/where 三个魔法端点。
- 全局根 `/` 同时保留 `/agents` + 旧 `/search`、`/index`、`/where`，路由统一走 MagicEntry。

### TODO-25 Agent-scoped 读写语义 + 向后兼容（`dispatcher.rs`）
- 全局路径 `/search`、`/index`、`/where` 映射到 default agent `agent_id=""`，行为与 init-pro-3 完全一致。
- `/agents/<id>/...` 路径以对应 `agent_id` 访问 DB / 魔法注册表，与全局路径互不可见（namespace 隔离）。
- 写入缓冲（per-fileid）天然按 fileid 隔离；agent-scoped fileid 保证不同 agent 的并发写入不串扰。

### TODO-26 集成测试（`tests/agent_namespace.rs`）
- 新增 `tests/agent_namespace.rs`（12 个用例）：DB schema 迁移、复合唯一约束、
  `list_agents()`、同名 key 跨 agent 不碰撞、`/agents` 目录枚举、per-agent search/index/where 隔离、
  全局路径向后兼容、scoped fileid 不串扰、default agent 归属等。

### TODO-27 文档
- 更新 README「状态」与本 changelog。
- 新增 `plans/init-pro-4/`（index + TODO-21..27）。

## 验收

| 验收点 | 测试 | 文件 |
|---|---|---|
| v0→v1 自动迁移，旧记忆归入 default agent | `migration_carries_legacy_into_default_agent` | tests/agent_namespace.rs |
| 复合唯一约束：跨 agent 同名 key 不碰撞 | `same_name_key_isolated_across_agents` | 同上 |
| `list_agents()` 枚举已存在 agent | `list_agents_returns_existing` | 同上 |
| `/agents` 目录枚举 agent | `agents_dir_lists_existing_agents` | 同上 |
| per-agent search/index/where 隔离 | `per_agent_index_isolated` / `per_agent_search_isolated` | 同上 |
| 全局路径映射 default agent 且向后兼容 | `global_paths_map_to_default_agent` | 同上 |
| scoped fileid 跨 agent 不串扰 | `scoped_fileid_distinct_per_agent` | 同上 |

- 全量回归：`cargo test --workspace` → **62 passed; 0 failed**（默认）
- fastembed 回归：`cargo test --workspace --features embed-fastembed` → **66 passed; 0 failed**
- clippy：`cargo clippy --workspace --all-targets -- -D warnings` → 零警告
- 行数：`dispatcher.rs` 620 ≤ 800；`magic.rs` 250 ≤ 400；`db/mod.rs` 556 ≤ 800

## Impact Surface

- **对用户可感知**：可在 `/agents/<agent_id>/` 下挂载各自隔离的记忆视图；多个 agent 同名记忆不再互相覆盖。
- **向后兼容**：全局 `/search`、`/index`、`/where` 路径语义不变（映射 default agent），既有客户端无感知。
- **无 schema breaking change**：v0→v1 自动迁移，旧库既有记忆归入 default agent，无需手工干预。
- **不影响**：NFS 协议边界、向量 KNN / FTS5 检索算法、写入 offset 拼接正确性（init-pro-3 成果保留）。
- **回归面**：魔法路径读写 + DB 查询全部按 agent 参数化；`tests/agent_namespace.rs` + 既有 magic 测试双重覆盖。
- **已知未覆盖**：鉴权（auth）不在本迭代范围，`agent_id` 仅作命名空间标识，不校验调用方身份。

## Related Docs

- [plans/init-pro-4/index.md](../../plans/init-pro-4/index.md)
- [既有 changelog：init-pro-3](./init-pro-3.md)
