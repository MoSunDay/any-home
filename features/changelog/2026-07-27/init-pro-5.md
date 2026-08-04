# init-pro-5：Agent 身份鉴权（TODO-28..34）

## 背景

`init-pro-4` 交付了多 agent **逻辑隔离**（`/agents/<id>/...` 按 agent 分片 DB 与 fileid），但 README「已知限制」
明确指出「**不包含鉴权**……恶意 agent 可冒充他人 agent_id」——且 `init-pro-3`、`init-pro-4` 两轮均将鉴权显式推迟。
本轮补齐这一缺口：让访问 `/agents/<id>/...` 的调用方**证明身份**。

设计原则延续项目哲学：**NFS-native**（尽量利用 NFSv3 原生 AUTH_SYS）、**plain files, no SDK**
（配置走 CLI/文件）、**向后兼容**（未配置鉴权时与 `init-pro-4` 行为完全一致）。

## TODO-28 spike 结论（关键架构决策）

调研 vendored `link_repos/nfsserve` 后确认：

- nfsserve **已**在每个 RPC 中把 AUTH_SYS 凭据解析进内部 `RPCContext.auth`（`rpcwire.rs::handle_rpc`），
  但 `NFSFileSystem` trait 的所有方法**只接收 fileid/filename、不接收任何调用方身份**；
- `RPCContext`、`auth_unix` 字段与模块均为**私有**，外部 VFS 完全无法触及；
- 每个 RPC 在独立 `tokio::spawn` 任务中执行，且 trait 方法在该任务内 **inline `.await`**（非再 spawn），
  故 **task-local** 方案可行：在 `handle_rpc` 注入身份，VFS 内读取。

因 nfsserve 为本地内嵌依赖，采用**最小化 fork**（而非改变 trait 签名）：

1. `rpc.rs`：`auth_unix` 字段加 `pub`。
2. 新增公共 `identity` 模块：`CallerIdentity{uid,gid,machinename,client_addr}` + task-local `CALLER` + `current_identity()`（非 server 环境返回默认/匿名）。
3. `rpcwire.rs`：`handle_rpc` 解析 auth 后，将身份快照 `CALLER.scope(...)` 包住 dispatch 段。

## 变更

### TODO-28 nfsserve 最小化 fork（`link_repos/nfsserve`）
- `src/rpc.rs`：`auth_unix` 五个字段改 `pub`。
- `src/identity.rs`（新，公开模块）：`CallerIdentity` + `task_local!(pub CALLER)` + `current_identity()`；含 2 个单测（server 外默认值、scope 内可见）。
- `src/lib.rs`：`pub mod identity;`。
- `src/rpcwire.rs`：`handle_rpc` 在解析 AUTH_SYS 后构造 `CallerIdentity` 快照，用 `CALLER.scope` 包住 NFS/MOUNT/portmap dispatch，使 VFS 实现可在调用期间读取调用方身份。
- **可复现性**：因 `link_repos/` 被 gitignore、nfsserve 由 `scripts/bootstrap.sh` 从上游 pin clone，本 fork 改动以补丁形式纳入 anyhome 仓库——`scripts/patches/nfsserve-auth-identity.patch`，bootstrap.sh 检出后经 `git apply` 幂等应用（以 `src/identity.rs` 为 marker，漂移即硬失败）。已在「干净上游 + 补丁」环境下验证 build/test 全绿。

### TODO-29 Agent 身份模型（`crates/anyhome/src/auth.rs`，新）
- 纯函数式定义：
  - `CallerCred{uid}` —— 来自 AUTH_SYS；`uid==0` 表匿名（无 AUTH_SYS 凭据，或 server 外测试环境）。
  - `AuthPolicy`：`Open`（默认，鉴权关闭）/ `Enforced{allow: HashMap<uid, BTreeSet<agent_id>>}`。
  - `AgentCred{agent_id, uid}` + `FromStr`（`"<id>:<uid>"`）+ `parse_clap`（clap value parser）。
- `parse_creds_file` / `parse_creds_text`：每行一个 `id:uid`，支持 `#` 行内注释与空行。
- `AuthPolicy::from_creds(&[])` → `Open`（空凭据即关闭鉴权，向后兼容的坍缩点）。

### TODO-30 鉴权策略（`auth.rs`）
- `AuthPolicy::permits(caller, agent_id)`：
  - `Open` → 恒真；
  - `Enforced` → 默认 agent（`""`）**始终公开**；具名 agent 要求调用方 `uid` 已注册该 agent；匿名（uid 0）仅可访问默认 agent。
- 11 个单元测试覆盖：开放策略、空凭据坍缩、注册 uid 放行/越权拒绝、默认 agent 公开、匿名受限、一 uid 多 agent、解析容错、uid 0 绑定拒绝。
  - `AgentCred::from_str` 拒绝 `uid 0` 绑定（AUTH_SYS 下 root 与 AUTH_NULL 不可区分，绑定 uid 0 会让所有匿名调用方获得该 agent 访问权）。

### TODO-31 Dispatcher 鉴权拦截（`dispatcher.rs`）
- `AnyHomeFs` 新增 `auth: AuthPolicy` 字段（默认 `Open`，故 `AnyHomeFs::new(...)` 既有签名与全部既有测试**零改动**通过）。
- 新增 `with_auth(self, policy)` builder、`current_caller()`（读 `nfsserve::identity::current_identity().uid`）、`require_agent(agent_id)`（不通过返回 `NFS3ERR_ACCES`，`Open`/默认 agent 下为 no-op）。
- 拦截点（每处先恢复 agent_id 再校验）：
  - `lookup`：`/agents/<id>` 入口、agent 子目录、魔法目录文件；
  - `read` / `write`：经 `index_name_for` / registry 恢复 agent_id（**含 NFS 句柄缓存/泄露场景**仍被拦）；
  - `create` / `create_exclusive` / `remove`；
  - `readdir`：`/agents` 按权限**过滤**（隐藏调用方不可见的 agent）、agent 子目录与魔法目录校验。
- 鉴权逻辑完全内聚于 `src/auth.rs`，dispatcher 仅委托 `require_agent`；`getattr`/`setattr`（纯元数据）未拦截以避免破坏 NFS 客户端缓存行为，已记入已知限制。

### TODO-32 CLI / 配置接线（`cli.rs` + `main.rs`）
- `--agent-uid <ID:UID>`（可重复）+ `--agent-creds <FILE>`（每行 `id:uid`，支持 `#` 注释）。
- `main.rs::build_auth_policy` 合并两源 → `AuthPolicy` → `AnyHomeFs::with_auth(...)`；空则 `Open` 并打日志。

### TODO-33 集成测试（`tests/agent_auth.rs`，新，9 用例）
- `as_caller(uid, fut)` 用 `CALLER.scope` 模拟不同 AUTH_SYS 调用方。
- 覆盖：向后兼容（Open）、冒充在 lookup 被拒、跨 agent 读/写拒绝（含泄露句柄）、跨 agent 命名空间 lookup 被拒、匿名受限默认 agent、默认 agent 始终公开、`/agents` 列表隐藏、一 uid 多 agent、跨 agent 删除被拒。

### TODO-34 文档
- README：测试计数（62→81 默认 / 66→85 fastembed）、新增能力条目、init-pro-5 TODO 节、改写「已知限制」为 soft auth 说明。

## 验收

- `cargo build --workspace` ✓（默认 + `--features embed-fastembed`）。
- `cargo clippy --workspace --all-targets -- -D warnings` ✓ 零警告（默认 + fastembed）。
- `cargo test --workspace` = **82 passed**（62 既有 + 11 auth 单测 + 9 auth 集成）；`--features embed-fastembed` = **86 passed**。既有用例**零回归**（鉴权默认关闭）。
- 行数：新增文件 ≤400（`auth.rs` 262 / `agent_auth.rs` 296 / `identity.rs` 75），迭代文件 ≤800（`dispatcher.rs` 666）。

## 风险与权衡

- **AUTH_SYS 可伪造**：`uid`/`gid` 由客户端自报，非密码学安全 → 定位为 **soft auth**（防误操作 + 防意外串扰），文档明确标注；强安全需 TLS/mTLS 前置或 RPCSEC_GSS/Kerberos。
- **vendored nfsserve 改动**：改动量极小（字段加 `pub` + 一个公开模块 + 一处 scope 包裹），不改变 `NFSFileSystem` trait 签名，nfsserve 自身单测仍通过。
- **向后兼容**：`AuthPolicy::default()` = `Open`，未配置凭据时 `require_agent` 恒为 no-op，确保既有部署/测试零行为变化。

## Related Docs

- [plans/init-pro-5/index.md](../../../plans/init-pro-5/index.md)
- [既有 changelog：init-pro-4](../2026-07-26/init-pro-4.md)
