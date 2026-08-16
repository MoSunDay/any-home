# TODO-57 · nfsserve identity 契约移植（2 例）

**状态**：done（commit 9926f18）

## 范围决策
nfsserve 上游**唯一**测试是 `src/identity.rs` 底部内联 2 例（其余 src 文件零
测试）。`CALLER` task_local 为 pub，可逐行移植。

## 变更
`tests/nfsserve_identity.rs`（34 行）：
- `defaults_outside_server`：RPC 外 `current_identity()` = Default（uid/gid 0、
  machinename/client_addr 空、is_anonymous）。
- `scoped_identity_visible`：`CALLER.scope` 内 uid/machinename 可见、非匿名。

## 意义
`src/auth.rs` 的 soft-auth 完全依赖这个 task-local 契约（RPC 外匿名回退 +
scope 内身份可见）；移植后该契约由 anyhome 自己的 CI 看护，不再依赖上游仓库。
