# TODO-11 `--export` 接线 + mount

## 目标
把 CLI `--export` flag 接到 nfsserve 的 `with_export_name()`，
使导出路径可配置（默认 `/`）。

## 核心实现
- `cli.rs`：已有 `--export`（默认 `/`）。
- `main.rs`：调 `listener.with_export_name(&cli.export)`
  （nfsserve `tcp.rs:171` `with_export_name(&mut self, &str)`）。
- `scripts/e2e.sh` 用 `EXPORT` 变量。

## 验收手段
- 编译通过。
- `cargo test` 通过。
- `scripts/e2e.sh` 用 `EXPORT` 变量。

## 状态
done

## 证据
- build clean。
- anyhome_magic 9/9 passed（dispatcher 级 e2e）。

## 卡点
无。

## 依赖
init-pro TODO-06。
