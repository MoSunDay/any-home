# init-pro-12 · 部署审计 + 魔法面缺陷 A/B/C/E 修复 + 6 e2e facets + 受控再部署回归（TODO-67..71 全部完成）

## Goal

在**不动任何对外语义契约**（`MemoryDb`/`MagicRegistry` 公共签名、每个 WRITE RPC
写穿 DB、每次数据变更单次 embed、`fs.db.*` 直连路径与 schema 全部不变）的前提下：
① 对已部署实例做只读审计，拿到字节级 ground truth；② 修复取证确认的四个魔法面
缺陷（TODO-70 的 A/B + 本轮新发现的 C/E）；③ e2e 补齐 6 个此前未覆盖魔法面的
facet；④ 在旧二进制上先跑数据安全回归，再受控重启到新 release 二进制跑全量
B 矩阵；⑤ 文档三件套 + 计数对账。**不重新规划**，按既定 init-pro-12 执行。

## 摸底结论（取证先行）

1. **部署实例 ground truth**（TODO-67）：`anyhome.db` 恰有 2 条记忆
   （`""`/`k` 31B、`""`/`smoke-pro11` 25B），单事务 dump 至
   `tmp/audit67_dump.json`；`anyhome-fs/` 为空（无污染需清理）。
2. **缺陷 A**：SEARCH/WHERE getattr 报虚拟 1MiB → NFSv3 客户端按 attr size
   对读缓冲 NUL 补齐（RFC 1813），`wc -c`/`cmp` 语义被污染。
3. **缺陷 B**：覆写短文本后残留旧尾部字节（替换路径未清 stale tail）。
4. **缺陷 C（本轮新发现）**：`RUST_LOG=nfsserve=debug` + strace 实锤——GNU
   `mkdir -p` 不做逐分量 LOOKUP（chdir 技巧后发相对 `mkdir("p")`），无守卫的
   `mkdir(ROOT_ID, "agents")` 落穿真实 FS，真实目录 shadow 虚拟目录，后续所有
   agent 遍历走 real。此前疑似的"缺陷 D"只是 C 的下游症状，非独立 bug。
5. **缺陷 E（本轮新发现，facet 6 触发）**：`MemoryDb::search` 先对**全局**
   `vector_top_k` 取 k 再按 agent 过滤——他 agent 更近的行吃满 k 槽，本 agent
   `search_k1` 空返回（bisect 复现链 `tmp/repro_sk10..13.sh` 的 v8 变体锁定）。
6. **环境坑**：挂载设备必须 `127.0.0.1:/`（port 只进 opts，写成
   `127.0.0.1:PORT:/` 会把 dirpath 发成 `"PORT:/"` → MNT3ERR_NOENT）；CLI 是
   `--db-path`/`--embedder dummy|fastembed`；`pkill -f` 自己的命令行会杀自身
   shell；fastembed 需 `HF_HOME`+`HF_HUB_OFFLINE=1`（网络不通）。

## 变更

1. **TODO-67 部署审计**：单 sqlite 事务 dump 记忆/FTS 计数/文件系统状态，
   产物 `tmp/audit67_dump.json` 作为 TODO-69 字节 diff 基准。
2. **TODO-70 缺陷 A/B + truncate**：getattr 真实尺寸；SETATTR(size)/O_TRUNC
   真截断并写穿；remove 失败记日志。钉死旧行为的单元测试按红线翻转。
3. **TODO-68 缺陷 C/E + 6 facets**：保留名守卫全套（EXIST/INVAL/NOTSUPP +
   readdir 隐藏）；search 改按 agent 分区精确 KNN；e2e 6 facets 全绿；
   `tests/magic_mkdir_guards.rs` 7 例、`db_vector` +1 饥饿回归例。
4. **TODO-69 受控再部署回归**：旧二进制 phase A（A1..A7）PASS → release 重建
   → 受控重启（pid 215315 → 701435，flags 逐字相同）→ 新二进制 phase B
   （B1..B10）PASS；2 条部署记忆全程字节不变；regress-12-* 三重残留检查归零。
5. **TODO-71 文档**：本目录五份 TODO 记录 + changelog + README 计数对账
   （203→217 / 207→221）。

## 红线自检

- `MemoryDb`/`MagicRegistry` 公共签名：未动（search 仅换实现）。
- 写穿语义：每个 WRITE RPC 仍单次写 DB、单次 embed；truncate/O_TRUNC 同。
- `fs.db.*` 路径与 schema：未动。
- 既有 e2e 断言：只增不弱；被翻转的单元测试全部是**钉死坏行为**的钉子，
  翻转处均留注释说明旧语义与翻转原因。

## 取证基线

收尾 `cargo test --workspace` = **217 passed / 0 failed**；`--features
embed-fastembed` = **221 passed**；clippy `--all-targets` 零警告；
`scripts/e2e.sh --require-mount 21111` exit 0（6 facets + 重启持久化 + auth +
export 强制全绿）。部署实例 pid 701435 运行修复版 release 二进制于 11111。
