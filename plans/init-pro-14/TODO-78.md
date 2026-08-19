# TODO-78 · init-pro 首轮 TODO-01..07 文档状态回填

## 背景

首轮计划（`plans/init-pro/`）的 TODO-01..07 早已随 MVP commit `169b627`
（2026-07-19）一次性全部落地，但七份 TODO 文档的「状态」栏始终停在
`pending`、证据栏是占位符——首轮欠下的文档账，与仓库实际进度脱节。
本 TODO 只清这笔账，**技术内容零改动**。

## 产出

- `plans/init-pro/TODO-01.md` .. `TODO-07.md`（共 7 份）：`## 状态`
  pending→done，`## 证据` 逐份回填，每份统一标注
  「（2026-08-18 回填，init-pro-14 TODO-78：首轮文档状态欠账，技术内容
  零改动）」，证据链指回 MVP commit `169b627`（工作区拓扑、commit gate
  自记 "cargo test 19 pass / clippy 0 / build clean"）及后续迭代的 live
  记录（如 init-pro-13 TODO-76 的 `--require-mount` 真实 mount 全绿）。
- 只动 `状态`/`证据` 两节：目标、核心实现、验收手段、卡点各节原文未动；
  不触碰任何代码、DB、部署物。

## 验收手段与证据

- 七份文件 diff 均仅覆盖 `## 状态` 与 `## 证据` 两节（工作树内可复核）。
- 今日基线复跑（init-pro-14 回归，见 TODO-79）：`cargo test --workspace`
  全绿——回填所断言的「工作区拓扑仍成立」成立。

## 状态

done
