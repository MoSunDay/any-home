# TODO-66 · 文档三件套 + 计数对账

**状态**：done

## 变更
- `features/changelog/2026-08-16/init-pro-11.md`（背景/变更 5 项/实测钉死的关键
  语义/计数对账/遗留与后续）。
- `plans/init-pro-11/`：index（Goal/摸底结论/红线/D1-D7/状态总表/计数对账）+
  TODO-62..66。
- README.md / README_EN.md：计数 149→203、153→207（各两处）；结构行集成测试
  16→20 文件（各一处）。

## 对账（当次实跑）
- `cargo test --workspace`：**203 passed / 0 failed / 0 ignored**
  （149 基线 + 54 新增）。
- `cargo test --features embed-fastembed`：**207 passed**（153 + 54）。
- 新增 54 = auth 单元 7 + 查询语法 21 + unicode 10 + 表达式深度 7 +
  hostile-text 9；集成测试文件 16 → 20。
- 验收复跑：workspace 连续 3 次全绿；每个新测试文件连续 20 次全绿；clippy
  `-D warnings` 双 feature 组合零警告；新文件 ≤400 行、src ≤800 行；
  无新增 `#[ignore]`。
