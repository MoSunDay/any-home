# TODO-29 Agent 身份模型

## 目标
新增 `crates/anyhome/src/auth.rs`，以纯函数式定义调用方凭据、鉴权策略与配置解析，为后续 dispatcher 拦截提供数据基础。

## 核心实现
- 纯函数式定义：
  - `CallerCred{uid}` —— 来自 AUTH_SYS；`uid==0` 表匿名（无 AUTH_SYS 凭据，或 server 外测试环境）。
  - `AuthPolicy`：`Open`（默认，鉴权关闭）/ `Enforced{allow: HashMap<uid, BTreeSet<agent_id>>}`。
  - `AgentCred{agent_id, uid}` + `FromStr`（`"<id>:<uid>"`）+ `parse_clap`（clap value parser）。
- `parse_creds_file` / `parse_creds_text`：每行一个 `id:uid`，支持 `#` 行内注释与空行。
- `AuthPolicy::from_creds(&[])` → `Open`（空凭据即关闭鉴权，向后兼容的坍缩点）。

## 验收手段
`cargo build --workspace` + `cargo clippy --workspace --all-targets -- -D warnings` ✓；`auth.rs` 单测（见 TODO-30）覆盖。

## 状态
done

## 证据
`crates/anyhome/src/auth.rs`（262 行 ≤ 400）；`from_creds(&[])` 返回 `Open`。

## 依赖
TODO-28
