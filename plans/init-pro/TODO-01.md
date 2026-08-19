# TODO-01 工作区骨架 + 依赖编译验证

## 目标
建可编译 workspace，`anyhome` crate 同时 path 依赖 nfsserve + libsql(core-only)，
`cargo build` 干净通过——证明依赖拓扑成立、编译时间可接受。

## 核心实现
- 根 `Cargo.toml`：`[workspace] members=["crates/anyhome"]`（**不列** link_repos，
  规避「package belongs to two workspaces」）；`resolver="2"`；
  `[workspace.dependencies]` 含 nfsserve、
  `libsql={path="../link_repos/libsql/libsql",default-features=false,features=["core"]}`、
  tokio(features full/rt-multi-thread/tracing)、anyhow、tracing、tracing-subscriber、clap(derive)。
- `rust-toolchain.toml`：`channel="1.85.0"`（对齐 + 可复现；实际 >=1.85 可用）。
- **不创建** `.cargo/config.toml`（core-only 不需 tokio_unstable，3 轮验证）。
- `crates/anyhome/{Cargo.toml,src/lib.rs}`：lib.rs 占位 `pub fn placeholder() {}`。
- `scripts/bootstrap.sh`：记 clone + **钉 commit** checkout + 叠 memory commit：
  - libsql: `git checkout 6f451a1fabacbcbc9960b232b4c1605a5021979b` (base) -> memory commit `faf79716...` 在其上
  - nfsserve: `git checkout c00c3614184bc79010c0aa3b69c2cdde1c5fca77` (base) -> memory commit `abe65c59...` 在其上

## 验收手段
1. `cargo check --workspace`（先 check 省时）退出 0
2. `cargo build --workspace` 退出 0 + 记录耗时
3. `cargo tree -p anyhome | grep -E "tonic|hyper|rustls|tower|aws-sdk-s3"` -> **必须无输出**（证明 core-only）
4. `rustc --version` = 1.85.0（rust-toolchain 生效）

## 状态
done

## 证据
- （2026-08-18 回填，init-pro-14 TODO-78：首轮文档状态欠账，技术内容零改动）
- MVP commit `169b627`（2026-07-19）一次落地 TODO-01..07：工作区拓扑与计划一致——members 仅 `crates/anyhome`、`rust-toolchain.toml` 钉 1.85.0、`scripts/bootstrap.sh` 钉 commit checkout，commit gate 自记 "cargo test --workspace (19 pass) / clippy -D warnings (0) / build clean"。
- 今日基线复跑（init-pro-14）：`cargo test --workspace` 全绿即工作区拓扑仍成立。

## 卡点
- 首次冷构建 libsql-ffi 的 bundled `sqlite3.c`（269269 行）单 cc 调用，预计数分钟。
- 构建机硬前置**仅需**：cc/gcc（实测 gcc 11.4.0 ✓）、network（crates.io 可达 ✓）、rustc 1.85（已装 ✓）。
- **不需要** cmake/libclang/bindgen/make（default 路径走预生成 `bundled/bindings/bindgen.rs` + `cc::Build`；
  cmake 仅 `multiple-ciphers` feature 用；bindgen 仅 `LIBSQL_DEV` 环境变量时触发——两者 core-only 都不开）。

## 依赖
无
