# TODO-67 · 部署实例只读审计（ground truth 取证）

## 目的

在任何修改之前，把已部署实例（pid 215315，release 旧二进制，端口 11111）的状态
固化为字节级基准，供 TODO-69 重启前后做 diff——**任何部署记忆字节变化都是回归**。

## 方法

1. **单 sqlite 事务** dump `anyhome.db`：`memories` 全行（agent_id/key/text/
   emb 维度）、`memories_fts` 计数、表清单——一次事务保证 WAL 视图一致，
   避免跨查询写穿插。
2. 挂载只读走查：根 readdir、`/index`、`/where` 可达性；`anyhome-fs/` 实际
   内容枚举（确认空目录，无 shadow 污染需要清理）。
3. 进程态记录：pid、启动 flags（`--fs-root ./anyhome-fs --db-path ./anyhome.db
   --addr 127.0.0.1:11111`）、`--embedder` 缺省 = Dummy/dim 64。

## 结论

- 部署记忆恰 2 条，均属 `agent_id=''`：`k`（31B）、`smoke-pro11`（25B）。
- `anyhome-fs/` 为空 → 后续回归无需清理历史污染。
- 产物：`tmp/audit67_dump.json`（phase A/B 的字节 diff 基准，B1/B9 复用）。
- 审计全程只读，未写任何 NFS/DB 状态。
