# TODO-73 · 优雅停机 + WAL checkpoint

## 产出

- `src/db/mod.rs` 172→199（+27）：`MemoryDb::checkpoint()` —— 对文件型 DB
  执行 `PRAGMA wal_checkpoint(TRUNCATE)` 并返回 `(busy, log, checkpointed)`；
  `:memory:` DB 直接返回 `(0,-1,-1)` 无害 no-op。只增签名，不改既有 API。
- `src/main.rs` 101→132（+31）：`tokio::select!` 同时等服务错误与
  SIGINT/SIGTERM（`tokio::signal::unix::signal`）；收到信号先记
  `SIGTERM received: starting graceful shutdown`（或 SIGINT 变体），执行
  checkpoint 并记 `WAL checkpoint complete (...)`，随后正常返回（退出码 0）。
- `tests/durability.rs` 47→112（+65，+3 例）：
  - `checkpoint_truncates_wal_to_zero_bytes`：文件 DB 写入后 `-wal` 非空，
    checkpoint 后 `-wal` 恰为 0 字节、`busy=0`。
  - `checkpointed_data_survives_reopen`：checkpoint 后关库重开，数据完好。
  - `memory_db_checkpoint_is_noop`：`:memory:` checkpoint 返回 `(0,-1,-1)`。

## 实测

- 单测 3 例全绿（默认组合）。
- live SIGTERM/SIGINT 演练在 TODO-76 中完成（见 TODO-76.md）：优雅日志 +
  WAL 2 665 672→0 B + 主库 4 096→94 208 B + 重启字节全等。

## libsql 语义钉死

`PRAGMA wal_checkpoint(TRUNCATE)` 成功时 libsql 报告 `(busy,log,checkpointed)
= (0,0,0)`——即便 WAL 实际有数百帧被搬回主库（隔离探针复现：wal 8 272→0 B、
主库增长、统计仍 `(0,0,0)`）。因此**字节尺寸才是收敛的权威证据**，日志中的
帧数仅作记录。`:memory:` 库上返回 `(0,-1,-1)`（WAL 未启用），与主线 SQLite
一致。
