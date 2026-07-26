# TODO-00 环境就绪（link_repos 重建，基线绿）

## 目标
重建 `link_repos/{nfsserve,libsql}`，确认 workspace 可编译、测试基线绿、核对 `NFSFileSystem` trait 方法集。

## 核心实现
- 原 `bootstrap.sh` pin 的"memory baseline"覆盖提交（`faf79716`/`abe65c59`）**从未发布到上游**，`git ls-remote` 不可达，克隆即失败。
- 经核对，二者"base"提交即**当前上游 HEAD**，anyhome 代码对上游 base 直接可编译 → 把 tip pin 改为上游 HEAD（`6f451a1f`/`c00c3614`），并加 `--depth 1`。
- 补齐沙箱缺的 `pkg-config` / `libssl-dev`（fastembed 传递依赖 openssl-sys 所需）。
- 核对 `NFSFileSystem` trait：`write` 带 offset；`create`/`create_exclusive`/`setattr`/`getattr`/`readdir` 均在 trait 内，无未实现必需方法；无独立 `fsync`/`access`/`mknod`。

## 验收手段
`scripts/bootstrap.sh` 幂等可复现；`cargo build --workspace`（默认 + `--features embed-fastembed`）通过；`cargo test --workspace` 基线 35 绿。

## 状态
done

## 证据
- `scripts/bootstrap.sh` HEAD 校验：libsql=`6f451a1f..`、nfsserve=`c00c3614..`。
- 修复 toolchain 安装被中断导致的 "Missing manifest" 后，默认构建 Finished。

## 依赖
无。
