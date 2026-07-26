# TODO-23 Agent-scoped fileid 编码

## 目标
让 fileid 携带 agent 维度，不同 agent 的同名记忆生成不同 fileid，彻底消除句柄跨 agent 串扰。

## 核心实现
- 引入 `scoped_name = "{agent_id}\0{name}"`（`\0` 分隔；合法记忆 name 不含 NUL，故无歧义）。
- fileid 仍由确定性 `FNV-1a(scoped_name)` 计算 → 同名不同 agent → 不同 fileid。
- 新增 `MagicEntry { agent_id: String, name: String, cat: Category }`，魔法注册表以 `MagicEntry` 为值。
- 新增 `CAT_AGENT`、`CAT_AGENT_DIR` 等 category 常量，支撑 `/agents` 多层路由。

## 验收手段
`tests/agent_namespace.rs`：`scoped_fileid_distinct_per_agent`（同名记忆在 default 与具名 agent 下 fileid 不同）；
`scoped_name_separator_unambiguous`。

## 状态
done

## 证据
同 name 在 `agent_id=""` 与 `agent_id="bot1"` 下算出的 fileid 不相等；反查 name 正确还原。

## 依赖
TODO-21。
