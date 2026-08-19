# init-pro-14 · 本地挂载 e2e 补全 cargo 内置 rust 实现 + 首轮文档状态回填（TODO-78..79 完成）

## Goal

把本地挂载 e2e 从「bash-only / manual」补全为 **cargo 内置的 rust 实现**：
`cargo test` 在可挂载主机上自动对真实内核 NFSv3 客户端跑通核心环路
（起真二进制 → mount.nfs 挂载 → 魔法路径断言 → SIGTERM 重启持久化），
不可挂载的沙箱按 e2e.sh 同款语义自动跳过、CI 保持绿。同时清偿首轮计划
的文档状态欠账（TODO-01..07 回填，技术内容零改动），随后全量回归。

## 变更

1. **TODO-78 文档状态回填**：`plans/init-pro/TODO-01..07.md` 状态
   pending→done + 证据回填（统一标注 2026-08-18 回填、技术内容零改动，
   证据链指回 MVP commit `169b627` 与后续 live 记录）。
2. **TODO-79 rust e2e**：新文件 `crates/anyhome/tests/e2e_mount.rs`
   （395 行，单测试，纯函数无 class）——e2e.sh 核心本地挂载环路的 cargo
   孪生：`CARGO_BIN_EXE_anyhome` 起真二进制（tempdir fs-root/db、自由端口
   + EADDRINUSE 重试）、`mount.nfs -o vers=3,tcp,nolock,noacl,port=P,
   mountport=P 127.0.0.1:/` 真挂载，断言错误导出拒挂、真实文件生命周期、
   /index→/search、/where(+prefix)、true-size、覆写/O_TRUNC、rm 三查
   （含直查 DB/FTS 行数）、unicode、/search_k、SIGTERM 优雅停机 +
   同库重启持久化。跳过语义对齐 e2e.sh（无 mount.nfs/非 root/非 Linux →
   skip；`ANYHOME_REQUIRE_MOUNT=1` 硬失败）；auth/agent 与 fastembed 的
   全量 live gate 仍归 `scripts/e2e.sh`。
3. **文档对账**：README 计数 222/234→223/235、tests 目录文件数、
   端到端小节补 rust e2e 一行；changelog
   `features/changelog/2026-08-19/init-pro-14.md`。

## 验收口径

- `cargo test --workspace` = **223** passed / 0 failed（+1 即 e2e_mount，
  cargo test 内完成真实 mount）。
- `cargo test --workspace --features embed-fastembed` = **235** passed /
  0 failed（fastembed_semantic 3 例无缓存自跳过，与之前一致）。
- clippy `-D warnings` 双组合零警告；`scripts/e2e.sh --require-mount
  21115` exit 0 全绿；rust e2e skip/require 双语义实弹验证。
- fastembed live e2e 本轮因 sandbox 出网受阻（hf 双源不可达），改验
  TODO-75 fail-fast 守卫（启动即报错不挂起）。
- 部署实例（pid 1077035、端口 11111）零接触。

## TODO 索引

- [TODO-78](TODO-78.md) 首轮 TODO-01..07 文档状态回填（已完成）
- [TODO-79](TODO-79.md) 本地挂载 e2e rust 实现 + 全量回归（已完成）
