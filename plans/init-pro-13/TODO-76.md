# TODO-76 · 受控再部署 + live e2e

## 步骤与结果

1. **Phase A（旧二进制，pid 701435）**：`tmp/regress76_phaseA.sh` ——
   部署记忆与 `tmp/audit67_dump.json` 字节全等、`regress-13-*` 写读/枚举/
   删除、真文件生命周期、操作后记忆复验、triple zero-residue。PASS。
2. **release 重建**：`cargo build --release --workspace`；二进制含优雅停机
   字符串（`grep -a` 验证）。
3. **受控换绑**：`kill -TERM 701435`（旧二进制无 handler，默认动作终止；
   数据安全由 `synchronous=FULL` 保证）→ 端口释放 → 同参数同日志文件
   （`anyhome-server.log`）起新二进制。
4. **迁移实弹验证（Phase B）**：`tmp/regress76_phaseB.sh` ——
   - 部署记忆重开后字节全等（2 行，31B/25B）；
   - `meta.schema_version=2`；`memories_vec_idx`/`..._shadow`/
     `..._shadow_idx` 全部消失；`libsql_vector_meta_shadow` 保留为空表
     （0 行）——与 TODO-72 取证预测一致；memories=2、emb 齐全；
   - `regress-13-*` 写读/枚举/删除 + DB/FTS/index 三零残留。PASS。
5. **SIGTERM 演练**：对 pid 971598 发 SIGTERM——
   - 日志实锤：`SIGTERM received: starting graceful shutdown` +
     `WAL checkpoint complete (TRUNCATE) ...`；
   - 字节证据：`anyhome.db-wal` 2 665 672→0 B、`anyhome.db` 4 096→94 208 B；
   - 重启后部署记忆与 audit dump 字节全等。
6. **live e2e**：`scripts/e2e.sh --require-mount 21111` exit 0（index/search/
   where/prefix/agent 隔离/true-size/覆写截断/rm 三查/真文件/unicode/
   search_k/重启持久化/auth/export 强制全绿）。

## 坑（实弹记录）

- **setsid 双 pid**：`setsid nohup ./anyhome ... &` 会 fork，首个 pgrep 抓到
  的是 wrapper pid；SIGTERM 打在 wrapper 上 → 无优雅日志、WAL 不动。定位
  真实服务必须 `pgrep -x anyhome`（真实 pid 971598，PPID=1 自建会话）。
- **二次启动 EADDRINUSE**：诊断期间的并发启动以 `Error: Address already in
  use` 退出码 1 干净失败，未污染数据（端口先绑、DB 幂等重开）。
- **checkpoint 统计**：libsql TRUNCATE 成功报 `(0,0,0)`，字节尺寸才是权威
  证据（见 TODO-73.md）。

## 收尾状态

部署实例 = 新二进制（release）、同参数（`--fs-root ./anyhome-fs --db-path
./anyhome.db --addr 127.0.0.1:11111`）、schema v2、WAL 已收敛、日志
`anyhome-server.log`、`regress-13-*` 零残留、回滚备份在
`tmp/audit72/anyhome.db*.bak`。
