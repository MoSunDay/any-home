# anyhome 实现计划 · init-pro-7

- **project**: init-pro-7（检索可控性 + 鉴权泄漏收口 retrieval controllability + auth leak closure）
- **created**: 2026-08-14
- **status**: done（TODO-40..45 全部完成）

## Goal

以 TODO-40 spike 为前提，收口 init-pro-5 鉴权遗留的**大小 oracle**，并补齐检索可控性：

1. **大小 oracle（安全缺口）**：`lookup` 在已授权调用方名下向进程级 registry 注册 `MagicEntry`，
   fileid 可离线推导（`"{agent}\0{key}"` 的 FNV-1a + CAT_INDEX），未授权调用方持 fileid 即可
   `getattr` 读到真实字节大小——违背 init-pro-5「泄漏句柄已收敛」语义（TODO-41 关闭）。
2. **`/search` top-k 不可控**：KNN 条数固定默认 16，路径无法表达（TODO-42 `/search_k<N>`）。
3. **`/where` 畸形查询不可区分**：spike 推翻旧注释「bad queries -> no rows」——畸形 FTS5 MATCH
   实际抛 libsql `SqliteFailure`（`fts5: syntax error near ...` / `unterminated string` /
   `unknown special query: ...`），NFS 层表现为不透明 `IO` 错误（TODO-43 分类 + 文本首行错误）。

## 关键技术决策

| 决策 | 结论 | 证据 |
|---|---|---|
| D1 metadata 鉴权面 | `getattr`/`setattr` **仅拦** agent 范围 CAT_INDEX 文件的 fileid（`require_agent` → `ACCES`）；目录 / search / where 虚拟文件 / 真实 FS 元数据不动 | `dispatcher.rs` getattr/setattr |
| D2 search_k 寻址 | k-salt fileid + registry 携带 k；N 严格数字、收敛 1..=64（MAX_K）；readdir 不枚举；`/search` 原样（k=16） | `magic.rs` |
| D3 /where 错误通道 | 错误经文本首行 `# error: invalid FTS query: <cond>` 编码（NFS READ 无 errno 通道）；参数绑定不动，注入安全性保持 | `db/mod.rs::WhereOutcome` + dispatcher 渲染 |
| D4（init-pro-5）例外收窄 | 原决策「getattr/setattr 从不鉴权」收窄为「**目录与真实 FS 元数据不拦截**」——`/index` 文件元数据纳入鉴权 | TODO-40 修正鉴权矩阵 |

## 修正鉴权矩阵（TODO-40 spike 产出）

| fileid kind | getattr / setattr（Enforced 下） |
|---|---|
| CAT_INDEX 文件，可解析 | 鉴权；未授权 → `ACCES` |
| CAT_INDEX 文件，不可解析（陈旧） | 零大小属性，公开 |
| 魔法目录 / search / where 虚拟文件 | 虚拟属性，公开 |
| 真实 FS | 不拦截 |
| 默认 agent | 始终公开 |

## 状态总表

| ID | 标题 | 状态 | 依赖 |
|----|------|------|------|
| TODO-40 | 现状 spike（大小 oracle 证实 + FTS 错误实测 + 修正鉴权矩阵） | done | — |
| TODO-41 | 关闭大小 oracle（getattr/setattr 拦 agent 范围 CAT_INDEX fileid） | done | TODO-40 |
| TODO-42 | `/search_k<N>`（top-k 可控，k-salt fileid，不枚举） | done | TODO-40 |
| TODO-43 | `/where` 畸形 FTS → `WhereOutcome::InvalidFts` + 首行错误文本 | done | TODO-40 |
| TODO-44 | 测试（auth_metadata 新 + where_fts/anyhome_magic 扩展） | done | TODO-41, TODO-42, TODO-43 |
| TODO-45 | 文档（changelog + plans + README 双语） | done | TODO-44 |

## 全局验收 gate
- TODO-40..45 全 done。
- `cargo build` / `cargo clippy --workspace --all-targets -- -D warnings` / `cargo test --workspace` 双 feature 全绿。
- `cargo test` = **102 passed**（87 既有 + 7 auth_metadata + 3 where_fts + 5 anyhome_magic）；
  `--features embed-fastembed` = **106 passed**。既有用例零回归。
- 行数：`dispatcher.rs` 760 / `magic.rs` 356 / `db/mod.rs` 650 / `auth_metadata.rs` 228（均 <400/<800 限内）。
