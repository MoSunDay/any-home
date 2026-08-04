# TODO-32 CLI / 配置接线

## 目标
将鉴权凭据通过 CLI 参数与文件两路接入 `AnyHomeFs`，空凭据则保持 `Open` 并打日志。

## 核心实现
- `--agent-uid <ID:UID>`（可重复）+ `--agent-creds <FILE>`（每行 `id:uid`，支持 `#` 注释）。
- `main.rs::build_auth_policy` 合并两源 → `AuthPolicy` → `AnyHomeFs::with_auth(...)`；空则 `Open` 并打日志。

## 验收手段
`cargo build --workspace` ✓ + `--features embed-fastembed` ✓；clippy 零警告。

## 状态
done

## 证据
`crates/anyhome/src/cli.rs`（`--agent-uid` / `--agent-creds`）+ `src/main.rs::build_auth_policy`。

## 依赖
TODO-31
