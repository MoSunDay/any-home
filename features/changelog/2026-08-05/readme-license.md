# 新增 README 英文版与 MIT 许可证

## 背景

仓库仅有中文 README，缺少英文版，国际用户无法快速了解项目。同时仓库缺少 LICENSE 文件，
开源许可不明确，阻碍他人使用与贡献。本次补齐这两项文档基础设施，并修复 README 中一处文档错误。

## 变更

### README 英文版与双语互链
- **`README_EN.md`**（新）：完整的英文 README 翻译，覆盖全部章节（设计理念、魔法路径、架构图、
  快速开始、CLI 参考、嵌入后端、状态、已知限制、项目结构）。`--dim` 默认值使用修正后的 64。
- **`README.md`**：顶部新增 `[English](README_EN.md) | [中文](README.md)` 语言切换链接。

### 文档错误修复
- **`README.md`**：`--dim` 默认值由错误的 `384` 修正为 `64`（与 `DUMMY_DIM = 64` 实际代码值一致）。

### MIT 许可证
- **`LICENSE`**（新）：标准 MIT License，版权持有者 `MoSunDay`。
- **`README.md`** / **`README_EN.md`**：标题下方新增 MIT 徽章 `[![License: MIT]](LICENSE)`。
- **`Cargo.toml`**：`[workspace.package]` 新增 `license = "MIT"`。
- **`crates/anyhome/Cargo.toml`**：`[package]` 新增 `license.workspace = true`（跟随 workspace 继承约定）。

## 测试覆盖

本次为纯文档/许可证变更，不涉及功能代码，无新增测试。

- 全量回归：`cargo test --workspace` → **82 passed, 0 failed**
- clippy：`cargo clippy --workspace --all-targets -- -D warnings` → **0 warnings**
- build：`cargo build --workspace` → **编译干净**
- 行数：`README_EN.md` 236 行 ≤ 400；`LICENSE` 21 行 ≤ 400

## Impact Surface
- 对用户/调用方可感知的影响：项目现为 MIT 许可，可自由使用；新增英文 README 方便国际用户。
- 不影响：CLI/Web/session/store/auth 等任何功能边界。

## Related Docs
- [既有 changelog：init-pro-5](../2026-07-27/init-pro-5.md)
