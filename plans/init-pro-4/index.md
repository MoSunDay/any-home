# anyhome 实现计划 · init-pro-4

- **project**: init-pro-4（多 Agent 命名空间隔离 multi-agent namespace isolation）
- **created**: 2026-07-26
- **status**: done（TODO-21..27 全部完成）

## Goal

在 `/agents/<agent_id>/...` 下提供**每个 agent 独立的 search/index/where 记忆视图**，
DB 与 fileid 编码全部按 agent 分片，消除多 agent 同名 key 碰撞与句柄串扰。**不含鉴权**（auth 留作后续方向）。

### 路径布局

```
/                                    (root)
├── agents/                          ← virtual directory
│   ├── <agent_id>/                  ← per-agent virtual directory
│   │   ├── search/                  ← magic: vector KNN (agent-scoped)
│   │   ├── index/                   ← magic: write-to-index (agent-scoped)
│   │   └── where/                   ← magic: FTS5 full-text (agent-scoped)
│   └── <agent_id>/ ...
├── search/                          ← preserved: maps to default agent (agent_id="")
├── index/
└── where/
```

### DB schema 变更

```
-- before (v0)
CREATE TABLE memory ( key TEXT PRIMARY KEY, text TEXT );

-- after (v1)
CREATE TABLE memory (
    agent_id TEXT NOT NULL DEFAULT '',
    key      TEXT NOT NULL,
    text     TEXT,
    PRIMARY KEY (agent_id, key)        -- composite unique
);
CREATE TABLE meta ( schema_version INTEGER );   -- v0→v1 auto-migration marker
```

### fileid 编码变更

```
-- before (init-pro-3)
fileid = FNV-1a(name)

-- after (init-pro-4)
scoped_name = "{agent_id}\0{name}"     -- \0 separator: legal names contain no NUL
fileid      = FNV-1a(scoped_name)      -- same name under different agents → different fileid
```

## 关键技术决策

| 决策 | 结论 | 证据 |
|---|---|---|
| D1 命名空间布局 | `/agents/<id>/{search,index,where}` 虚拟目录（**A**），保留全局根路径向后兼容 | `dispatcher.rs` 多层 lookup/readdir |
| D2 fileid 编码 | `scoped_name = "{agent_id}\0{name}"` + FNV-1a（**A**），\0 分隔无歧义 | `magic.rs` MagicEntry |
| D3 DB schema 迁移 | v0→v1 自动表重建，旧记忆归入 default agent `""`（**A**），无手工干预 | `db/mod.rs` schema_version |
| D4 鉴权 | **本轮不做**（defer），`agent_id` 仅作命名空间标识，不校验身份 | README 已知限制 |

## 状态总表 + 依赖图

| ID | 标题 | 状态 | 依赖 |
|----|------|------|------|
| TODO-21 | DB schema 迁移（agent_id 列 + 复合唯一 + v0→v1） | done | — |
| TODO-22 | DB 查询按 agent_id 参数化 + list_agents() | done | TODO-21 |
| TODO-23 | Agent-scoped fileid 编码（MagicEntry + CAT_AGENT） | done | TODO-21 |
| TODO-24 | /agents 虚拟命名空间多层路由 | done | TODO-22, TODO-23 |
| TODO-25 | Agent-scoped 读写语义 + 向后兼容 | done | TODO-24 |
| TODO-26 | 集成测试 tests/agent_namespace.rs（12 用例） | done | TODO-21..25 |
| TODO-27 | 文档 | done | TODO-21..26 |

```
TODO-21 ─┬─> TODO-22 ──┐
         └─> TODO-23 ──┴─> TODO-24 ──> TODO-25 ──> TODO-26 ──> TODO-27
```

## 全局验收 gate
- TODO-21..27 全 done。
- `cargo build --workspace` ✓ + `cargo build --workspace --features embed-fastembed` ✓
- `cargo clippy --workspace --all-targets -- -D warnings` ✓（零警告）
- `cargo test --workspace` = **62 passed**（默认；基线 45 + 新增/调整）
- `cargo test --workspace --features embed-fastembed` = **66 passed**
- 量化：跨 agent 同名 key fileid 不同且 DB 各自独立；全局路径读写与 init-pro-3 行为一致。

## 已锁定的开放决策
- **D1（命名空间布局）= `/agents/<id>/...` 虚拟目录**：层级清晰，全局根路径向后兼容。
- **D2（fileid 编码）= scoped_name + FNV-1a**：保持确定性 fileid 与 init-pro-3 的反查恢复能力，agent 维度自然纳入。
- **D3（schema 迁移）= 自动表重建**：复合唯一约束无法 `ALTER` 直接达成，重建拷贝保证旧数据归入 default agent。
- **D4（鉴权）= 本轮不做**：`agent_id` 是命名空间标识而非身份凭据，鉴权留作后续迭代。
