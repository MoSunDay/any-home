# anyhome 实现计划 · init-pro-5

- **project**: init-pro-5（Agent 身份鉴权 agent identity authentication）
- **created**: 2026-07-27
- **status**: done（TODO-28..34 全部完成）

## Goal

为 `/agents/<id>/...` 访问增加**调用方身份鉴权**：基于 NFSv3 原生 AUTH_SYS，让访问某个 agent 命名空间的调用方**证明身份**，
补齐 init-pro-4 README「已知限制」中显式推迟的鉴权缺口（恶意 agent 可冒充他人 agent_id）。

设计原则延续项目哲学：

- **NFS-native**：尽量利用 NFSv3 原生 AUTH_SYS（uid/gid/machinename），不引入额外鉴权协议；
- **plain files, no SDK**：配置走 CLI 参数 / `id:uid` 文本文件，不依赖 SDK；
- **向后兼容**：未配置鉴权凭据时行为与 `init-pro-4` **完全一致**（`AuthPolicy::default()` = `Open`）。

## 关键技术决策

| 决策 | 结论 | 证据 |
|---|---|---|
| D1 身份注入方案 | task-local `CALLER` scope（最小化 fork nfsserve，不改 trait 签名）—— 每个 RPC 在独立 `tokio::spawn` 任务中执行，task-local 可在 VFS 内读取调用方身份 | `link_repos/nfsserve/src/identity.rs` + `rpcwire.rs` |
| D2 鉴权策略模型 | `AuthPolicy{Open, Enforced{allow}}` 纯函数式，`from_creds(&[])`→`Open` 为向后兼容坍缩点 | `crates/anyhome/src/auth.rs` |
| D3 uid 0 语义 | uid 0 表示匿名（AUTH_NULL，无 AUTH_SYS 凭据 / server 外测试环境），不可绑定 agent（AUTH_SYS 下 root 与匿名不可区分） | `auth.rs` `FromStr` guard |
| D4 getattr/setattr | 纯元数据不做鉴权（避免破坏 NFS 客户端缓存行为），已记入已知限制 | changelog + README |

## 状态总表 + 依赖图

| ID | 标题 | 状态 | 依赖 |
|----|------|------|------|
| TODO-28 | nfsserve 最小化 fork（`link_repos/nfsserve`） | done | — |
| TODO-29 | Agent 身份模型（`crates/anyhome/src/auth.rs`，新） | done | TODO-28 |
| TODO-30 | 鉴权策略（`auth.rs`） | done | TODO-29 |
| TODO-31 | Dispatcher 鉴权拦截（`dispatcher.rs`） | done | TODO-30 |
| TODO-32 | CLI / 配置接线（`cli.rs` + `main.rs`） | done | TODO-31 |
| TODO-33 | 集成测试（`tests/agent_auth.rs`，新，9 用例） | done | TODO-31 |
| TODO-34 | 文档 | done | TODO-28..33 |

```
TODO-28 ──> TODO-29 ──> TODO-30 ──┬─> TODO-31 ──> TODO-32 ──> TODO-33 ──> TODO-34
```

## 全局验收 gate
- TODO-28..34 全 done。
- `cargo build --workspace` ✓ + `cargo build --workspace --features embed-fastembed` ✓
- `cargo clippy --workspace --all-targets -- -D warnings` ✓（零警告）
- `cargo test --workspace` = **82 passed**（62 既有 + 11 auth 单测 + 9 auth 集成）；`--features embed-fastembed` = **86 passed**。既有用例**零回归**（鉴权默认关闭）。
- 量化：未配置凭据时行为与 init-pro-4 完全一致；配置后未授权 uid 访问 `/agents/<id>/` 返回 `NFS3ERR_ACCES`。

## 已锁定的开放决策
- **D1（身份注入）= task-local `CALLER` scope + 最小化 fork nfsserve**：在 `handle_rpc` 注入身份、VFS 内读取，不改变 `NFSFileSystem` trait 签名，nfsserve 自身单测仍通过。
- **D2（策略模型）= `AuthPolicy{Open, Enforced{allow}}` 纯函数式**：`from_creds(&[])`→`Open` 是向后兼容的坍缩点，未配置凭据即关闭鉴权。
- **D3（uid 0 语义）= uid 0 表匿名，不可绑定 agent**：AUTH_SYS 下 root 与匿名不可区分，故禁止将 uid 0 绑定到具名 agent。
- **D4（getattr/setattr）= 纯元数据不拦截**：避免破坏 NFS 客户端缓存行为，已记入已知限制。
- **soft auth 限制（锁定）**：AUTH_SYS 的 `uid`/`gid` 由客户端自报，**非密码学安全** → 定位为 **soft auth**（防误操作 + 防意外串扰）；强安全需 TLS/mTLS 前置或 RPCSEC_GSS/Kerberos。
