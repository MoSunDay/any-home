# README 项目结构测试文件计数修正（11 → 12）

## 背景

init-pro-7 评审（review verdict: go-live ready）记录了 1 个非阻塞文档 nit：
README 双语「项目结构」处写「集成测试 11 个文件 / 11 files」，实际
`crates/anyhome/tests/` 已有 **12** 个测试文件（本轮 `auth_metadata.rs` 新增后
旧值 11 本应 +1，此前历史值就已滞后一轮）。本次顺手 repair-on-touch 修正。

## 变更

### README 测试计数修正
- **`README.md`**：项目结构树 `tests/` 注释由「集成测试（11 个文件）」修正为
  「集成测试（12 个文件）」。
- **`README_EN.md`**：同处由「integration tests (11 files)」修正为
  「integration tests (12 files)」。

## 测试覆盖

纯文档修正，不涉及功能代码，无新增测试。计数与 `ls crates/anyhome/tests/*.rs | wc -l`
当次实跑结果（12）一致。

- 全量回归：`cargo test --workspace` → **102 passed, 0 failed**
- 全量回归：`cargo test --workspace --features embed-fastembed` → **106 passed, 0 failed**
- clippy：`cargo clippy --workspace --all-targets -- -D warnings`（默认 + embed-fastembed）→ **0 warnings**
- build：`cargo build --workspace`（默认 + embed-fastembed）→ **编译干净**
- 行数：本次仅改动 2 行既有文档，无新增源文件

## Impact Surface
- 对用户/调用方可感知的影响：仅文档准确性（项目结构示意与实际文件数一致）。
- 不影响：CLI/Web/session/store/auth/NFS 协议行为等任何功能边界。

## Related Docs
- [init-pro-7 changelog](init-pro-7.md)（计数滞后的来源迭代）
- [既有 docs changelog：readme-license](../2026-08-05/readme-license.md)
