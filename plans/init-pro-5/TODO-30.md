# TODO-30 鉴权策略

## 目标
定义 `AuthPolicy::permits` 的鉴权判定语义，并补齐单元测试覆盖各分支。

## 核心实现
- `AuthPolicy::permits(caller, agent_id)`：
  - `Open` → 恒真；
  - `Enforced` → 默认 agent（`""`）**始终公开**；具名 agent 要求调用方 `uid` 已注册该 agent；匿名（uid 0）仅可访问默认 agent。
- `AgentCred::from_str` 拒绝 `uid 0`：AUTH_SYS 下 `uid 0` 同时表示匿名（AUTH_NULL）与 root，绑定它会让匿名调用方获得该 agent 访问权。
- 12 个单元测试覆盖：开放策略恒真、强制策略放行/拒绝、匿名仅默认 agent、默认 agent 始终公开、uid 0 不可绑定（`FromStr` guard）等分支。

## 验收手段
`auth.rs` 12 个单测全绿；`cargo clippy --workspace --all-targets -- -D warnings` 零警告。

## 状态
done

## 证据
`crates/anyhome/src/auth.rs::permits` + 内联 `#[cfg(test)]` 12 用例。

## 依赖
TODO-29
