# TODO-34 文档

## 目标
同步 README「状态/已知限制」与 init-pro-5 计划文档，记录 Agent 身份鉴权的设计与验收结论。

## 核心实现
- README：更新测试计数（62→81 默认 / 66→85 fastembed）、新增能力条目、补 init-pro-5 TODO 节、改写「已知限制」为 soft auth 说明（AUTH_SYS 可伪造，强安全需 TLS/mTLS 或 RPCSEC_GSS）。
- 新增 `features/changelog/2026-07-27/init-pro-5.md`（背景、TODO-28..34 变更、验收、风险与权衡）。
- 新增 `plans/init-pro-5/`：`index.md`（Goal、设计原则、D1-D4 决策、依赖图、验收 gate、锁定决策）+ `TODO-28..34.md`。

## 验收手段
文档与代码实际行为一致；行数预算达标（`auth.rs` 262 / `agent_auth.rs` 296 / `identity.rs` 75 ≤ 400；`dispatcher.rs` 666 ≤ 800）。

## 状态
done

## 证据
README 状态节、changelog、计划 index 与 TODO 互相对应，无悬空引用。

## 依赖
TODO-28..33
