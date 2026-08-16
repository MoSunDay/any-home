# TODO-55 · 文档三件套 + 计数对账

**状态**：done

## 变更
- `features/changelog/2026-08-16/init-pro-9.md`（新）：背景（取证基线）、五项
  变更、关键决策（契约红线 / 预防性拆分 / into_inner 权衡 / 最小 readdir 用例）、
  Impact Surface、验收数字（123/127 = 当次实跑）。
- `README.md` / `README_EN.md`：测试计数 109/113 → 123/127（各两处）；项目结构
  补 `dispatcher/` 子模块行、`db/` 描述更新为 schema/crud/query 子模块。
- `plans/init-pro-9/`：index（决策表 D1–D4 + 状态总表 + 验收 gate）+
  TODO-51..55。

## 对账
- changelog / README / plans 三处测试计数统一为 **123 / 127**（当次 `cargo test`
  实跑：109 基线 + 12 dispatcher 直连 + 2 锁毒恢复）。
- 行数（最终态）：`db/mod.rs` 172、`db/schema.rs` 273、`db/crud.rs` 126、
  `db/query.rs` 138（新文件均 <400 门）；`magic.rs` 543、`backend/realfs.rs` 436、
  `dispatcher.rs` 459、`dispatcher/{dbops,routing,readdir}.rs` 237/224/206
  （迭代文件均 <800 门）。
- 集成测试文件数不变（13）；新增测试均为内联 `#[cfg(test)]` 单测。
