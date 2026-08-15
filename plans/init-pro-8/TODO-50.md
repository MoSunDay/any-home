# TODO-50 · 文档三件套 + 记忆同步

**状态**：done

## 变更
- `features/changelog/2026-08-15/init-pro-8.md`（新）：背景（spike 基线）、四项变更、
  关键决策（契约红线 / clone 下界收缩 / 大文件取舍）、Impact Surface（实际组件面）、
  验收数字（109/113 = 当次实跑）。
- `README.md` / `README_EN.md`：已知限制中写缓冲描述更新（256 条目「任意淘汰」→
  「256 条目 + 8 MiB 双上限，LRU，单缓冲超限组合后丢弃」）；测试计数 102/106 →
  109/113；集成测试文件 12 → 13。
- `plans/init-pro-6/index.md`：D3 行追加 init-pro-8 演进注记（不重写历史决策）。
- `plans/init-pro-8/`：index（决策表 D1-D5 + 状态总表 + 验收 gate）+ TODO-46..50。

## 对账
- changelog / README / plans 三处测试计数统一为 **109 / 113**（当次 `cargo test` 实跑）。
- 行数：`dispatcher.rs` 455、`dbops.rs` 135、`routing.rs` 95、`readdir.rs` 152、
  `magic.rs` 490、`db/mod.rs` 663——新文件均 <400、迭代文件均 <800。
