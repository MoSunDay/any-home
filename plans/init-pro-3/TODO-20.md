# TODO-20 回归测试 + 文档

## 目标
为上述修复补集成测试，并更新 changelog / README / 计划文档。

## 核心实现
- 新增 `tests/magic_write_semantics.rs`（10 用例：offset 拼接、大文本、exclusive 冲突、ROFS、真实大小、重启句柄、二进制 INVAL）。
- 更新 `tests/anyhome_magic.rs`：`writing_to_search_*` 改为断言 `ROFS`。
- 修复 `scripts/bootstrap.sh`（pin 上游 HEAD + `--depth 1`）。
- 更新 `features/changelog/2026-07-26/init-pro-3.md`、`plans/init-pro-3/`、README「状态」。

## 验收手段
`cargo test --workspace` 全绿（45）；clippy 零警告；行数预算达标。

## 状态
done

## 证据
- `cargo test --workspace` = 45 passed。
- `dispatcher.rs` 461 行（≤ 800）；`magic.rs` 198、`db/mod.rs` 381（≤ 400）。

## 依赖
TODO-14..19。
