# TODO-61 · 文档三件套 + 计数对账

**状态**：done

## 变更
- `features/changelog/2026-08-16/init-pro-10.md`（背景/变更 6 项/关键决策/
  Impact Surface/验收当次实跑）。
- `plans/init-pro-10/`：index（Goal/红线/D1-D5/状态总表）+ TODO-56..61。
- README.md / README_EN.md：计数 123→149、127→153（各两处）；结构行集成测试
  13→16 文件（各一处）。

## 对账（当次实跑）
- `cargo test --workspace`：149 passed / 0 failed / 0 ignored。
- `cargo test --features embed-fastembed`：153 passed。
- 新增 26 = identity 2 + vector 12 + fts 12；文件数 13 → 16。
- 评审后复跑（TODO-58 fuzz 断言校准后）：`--test libsql_vector_semantics`
  22 连绿、`--workspace` 3 连绿（149）、`embed-fastembed` 全绿（153），
  计数与上表一致；vector 文件行数 332 → 342（注释增量）。
