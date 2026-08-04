# TODO-31 Dispatcher 鉴权拦截

## 目标
在 `dispatcher.rs` 的各文件操作入口接入 `require_agent` 鉴权，未授权访问返回 `NFS3ERR_ACCES`，同时保持既有调用零改动通过。

## 核心实现
- `AnyHomeFs` 新增 `auth: AuthPolicy` 字段（默认 `Open`，故 `AnyHomeFs::new(...)` 既有签名与全部既有测试**零改动**通过）。
- 新增 `with_auth(self, policy)` builder、`current_caller()`（读 `nfsserve::identity::current_identity().uid`）、`require_agent(agent_id)`（不通过返回 `NFS3ERR_ACCES`，`Open`/默认 agent 下为 no-op）。
- 拦截点（每处先恢复 agent_id 再校验）：
  - `lookup`：`/agents/<id>` 入口、agent 子目录、魔法目录文件；
  - `read` / `write`：经 `index_name_for` / registry 恢复 agent_id（**含 NFS 句柄缓存/泄露场景**仍被拦）；
  - `create` / `create_exclusive` / `remove`；
  - `readdir`：`/agents` 按权限**过滤**（隐藏调用方不可见的 agent）、agent 子目录与魔法目录校验。
- 鉴权逻辑完全内聚于 `src/auth.rs`，dispatcher 仅委托 `require_agent`；`getattr`/`setattr`（纯元数据）未拦截以避免破坏 NFS 客户端缓存行为，已记入已知限制。

## 验收手段
`cargo build --workspace` ✓；既有 `agent_namespace.rs` 用例零回归（鉴权默认 `Open`）；`dispatcher.rs` 666 行 ≤ 800。

## 状态
done

## 证据
`crates/anyhome/src/dispatcher.rs` 内 `require_agent` 委托点；`AnyHomeFs::with_auth` builder。

## 依赖
TODO-30
