# TODO-33 集成测试

## 目标
为 Agent 身份鉴权补端到端集成测试，覆盖向后兼容、跨 agent 拒绝、句柄泄露、匿名、`/agents` 过滤等场景。

## 核心实现
- `as_caller(uid, fut)` 用 `CALLER.scope` 模拟不同 AUTH_SYS 调用方。
- 新增 `tests/agent_auth.rs`（9 个用例），覆盖：
  - 向后兼容（Open）；
  - 冒充在 lookup 被拒；
  - 跨 agent 读/写拒绝（含泄露句柄）；
  - 跨 agent 命名空间 lookup 被拒；
  - 匿名受限默认 agent；
  - 默认 agent 始终公开；
  - `/agents` 列表隐藏（调用方不可见的 agent）；
  - 一 uid 多 agent；
  - 跨 agent 删除被拒。

## 验收手段
`cargo test --workspace` = **81 passed**（62 既有 + auth 单测 + 9 auth 集成）；`--features embed-fastembed` = **85 passed**；既有用例零回归。`agent_auth.rs` 296 行 ≤ 400。

## 状态
done

## 证据
9 个 `agent_auth` 用例全绿；未授权 uid 访问 `/agents/<id>/` 返回 `NFS3ERR_ACCES`。

## 依赖
TODO-31
