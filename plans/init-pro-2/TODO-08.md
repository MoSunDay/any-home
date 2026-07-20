# TODO-08 持久化 / 重启存活

## 目标
记忆数据持久化到 DB 文件，重启后存活；dim 持久化并在启动时校验（不匹配=硬错误，
防止向量索引损坏）；FTS5 索引在需要时自动重建。

## 核心实现
- `db/mod.rs` 新增 `meta` 表（key-value，存 `embedding_dim`）。
- `reconcile_dim()`：启动时比对 CLI dim 与 meta dim，不一致直接 `Err`。
- `ensure_fts_populated()`：需要时重建 FTS5 虚拟表内容。
- Connection 直接持有——libsql `Connection` 内部 `Arc`，自动 Send+Sync+Clone，
  **不包 Mutex**（沿用 init-pro TODO-03 决策）。

## 验收手段
`tests/db_persistence.rs`——3 个非 `#[ignore]` 硬 gate：
1. 重启后记忆存活（写入 -> 重新 open -> 记忆仍在）
2. dim 持久化（meta 表存住 embedding_dim）
3. dim 不匹配 = `Err`（CLI dim ≠ meta dim 时启动失败）

## 状态
done

## 证据
- `cargo test --workspace` 中 db_persistence 3/3 passed。
- `db/mod.rs` 349 行。

## 卡点
无（已完成）。

## 依赖
init-pro TODO-03（DB schema + MemoryDb）。
