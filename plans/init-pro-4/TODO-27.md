# TODO-27 文档

## 目标
同步 README「状态」、changelog 与计划文档，记录多 agent 命名空间隔离的设计与验收结论。

## 核心实现
- 更新 README「状态」：将"多 agent 命名空间隔离尚未实现"更新为已交付（命名空间隔离 done；鉴权仍为后续方向）。
- 新增 `features/changelog/2026-07-26/init-pro-4.md`（背景、TODO-21..27 变更、验收、Impact Surface）。
- 新增 `plans/init-pro-4/`：`index.md`（Goal、路径布局、schema/fileid 变更、D1-D4 决策、依赖图、验收 gate）+ `TODO-21..27.md`。

## 验收手段
文档与代码实际行为一致；行数预算达标（dispatcher.rs 620 ≤ 800；magic.rs 250 ≤ 400；db/mod.rs 556 ≤ 800）。

## 状态
done

## 证据
README 状态节、changelog、计划 index 与 TODO 互相对应，无悬空引用。

## 依赖
TODO-21..26。
