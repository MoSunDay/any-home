# TODO-28 nfsserve 最小化 fork

## 目标
在 vendored `link_repos/nfsserve` 中以最小改动注入调用方身份，使 VFS 实现可在 RPC 处理期间读取 AUTH_SYS 凭据，而不改变 `NFSFileSystem` trait 签名。

## 核心实现
- `src/rpc.rs`：`auth_unix` 五个字段改 `pub`。
- `src/identity.rs`（新，公开模块）：`CallerIdentity` + `task_local!(pub CALLER)` + `current_identity()`；含 2 个单测（server 外默认值、scope 内可见）。
- `src/lib.rs`：`pub mod identity;`。
- `src/rpcwire.rs`：`handle_rpc` 在解析 AUTH_SYS 后构造 `CallerIdentity` 快照，用 `CALLER.scope` 包住 NFS/MOUNT/portmap dispatch，使 VFS 实现可在调用期间读取调用方身份。

## 验收手段
改动以补丁形式纳入 anyhome 仓库——`scripts/patches/nfsserve-auth-identity.patch`，`scripts/bootstrap.sh` 从上游 pin clone 后经 `git apply` 幂等应用（以 `src/identity.rs` 为 marker，漂移即硬失败）；在「干净上游 + 补丁」环境下 build/test 全绿，nfsserve 自身单测仍通过。

## 状态
done

## 证据
`link_repos/nfsserve/src/identity.rs` + `rpcwire.rs::handle_rpc` 内 `CALLER.scope`；`scripts/patches/nfsserve-auth-identity.patch` 可幂等 apply。

## 依赖
—
