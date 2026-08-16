# init-pro-12：部署审计 + 魔法面缺陷 A/B/C/E 修复 + 6 e2e facets + 受控再部署回归（TODO-67..71）

## 背景

init-pro-11 收尾时留下两条线索：① 候选 TODO"magic 文件读 NUL 填充"（查询
文件报虚拟 1 MiB，真实内容后带 NUL 尾迹）；② 魔法面 e2e 仍有六块未覆盖
（index 回读尺寸、覆写/O_TRUNC、rm 三重、真实文件、unicode、search_kN）。
本轮先对部署实例做只读审计拿 ground truth，再逐缺陷取证修复，最后在旧/新
二进制上两段式回归并受控重启。红线不变：`MemoryDb`/`MagicRegistry` 公共
签名、每 WRITE 写穿 DB、每次数据变更单次 embed、`fs.db.*` 路径与 schema。
取证基线：`cargo test --workspace` 203 passed / `--features embed-fastembed`
207 passed。

## 变更

1. **TODO-67 部署审计**：单 sqlite 事务 dump 部署库 → `tmp/audit67_dump.json`
   （2 条记忆 `""`/`k`、`""`/`smoke-pro11`）；`anyhome-fs/` 确认空；后续所有
   字节 diff 的基准。
2. **TODO-70 缺陷 A/B + 截断**：
   - 查询文件（SEARCH/WHERE/INDEX）getattr 报**真实内容长度**，不再虚拟
     1 MiB——NFSv3 客户端按 attr size NUL 补齐读缓冲（RFC 1813），旧行为
     直接污染 `wc -c`/`cmp`。
   - 覆写短文本不再残留旧尾；SETATTR(size)/O_TRUNC 经 `magic::truncate_buffer`
     真截断并写穿 DB；remove 失败记日志不再静默。
   - 钉死旧行为的单测按红线翻转（注释留旧语义）：`magic_write_semantics`
     +4 截断例、`magic.rs` +3 helper 例。
3. **TODO-68 缺陷 C/E + 6 facets**：
   - **缺陷 C**：strace + `RUST_LOG=nfsserve=debug` 实锤 GNU `mkdir -p` 不做
     逐分量 LOOKUP，`mkdir(ROOT,"agents")` 落穿真实 FS、shadow 虚拟目录。
     修法：保留名（search/index/where/agents/search_kN）对创建类 RPC 主张
     NFS3ERR_EXIST 令客户端回退 lookup，remove/rename-from 回 INVAL，readdir
     根目录隐藏保留名；`tests/magic_mkdir_guards.rs` 7 例（含 `mkdir -p`
     全流程落库、零真实目录）。
   - **缺陷 E**：`MemoryDb::search` 原在全局 `vector_top_k` 之后才按 agent
     过滤——他 agent 更近的行吃满 k 槽，`search_k1` 空返回（facet 6 触发，
     bisect 链 v8 锁定）。改为按 agent 分区精确 KNN（`vector_distance_cos`，
     `id ASC` 破平）；`vector_top_k` 的 k=0 堆损坏 abort 不再可达。
     `db_vector` +1 饥饿回归例；`libsql_vector_semantics` 6 处旧钉翻转
     （距离序输出、id 破平、维度错配新措辞、k=0 进程内断言）。
   - e2e 6 facets（index 真实尺寸 / 覆写+O_TRUNC / rm 三重 / 真实文件 /
     unicode / search_kN 上限）全绿，DIAG 诊断块移除。
4. **TODO-69 两段式回归 + 受控重启**：旧二进制 phase A（A1..A7）PASS →
   release 重建 → 受控重启（215315 → 701435，flags 逐字相同）→ 新二进制
   phase B（B1..B10）PASS。部署记忆全程字节零变化；`regress-12-*` 三重残留
   检查（DB/FTS/FS）归零。踩坑：`pgrep -f` 命中 setsid 包装进程险杀错对象，
   改按 `/proc/<pid>/exe` 精确匹配。
5. **TODO-71 文档**：`plans/init-pro-12/` 六份 + 本 changelog + README
   计数对账。

## 实测钉死的关键语义

- **mkdir -p 守卫**：保留名 MKDIR/CREATE/SYMLINK → EXIST（客户端回退
  lookup 命中魔法 fileid）；`mkdir -p agents/<id>/index` 冷启动全流程落库、
  `anyhome-fs/` 零生长；readdir 隐藏历史 shadow 条目。
- **scoped KNN**：他 agent 的行无论多近都不占本 agent 的 k 槽；距离序输出、
  `id ASC` 确定性破平；k=0 → 空结果（不再 SIGABRT）。
- **真实尺寸**：`stat`/读回字节/DB 长度三方一致；空查询 = 0 字节。
- **截断写穿**：O_TRUNC 后 DB text = ''，跨重启持久。
- **部署连续性**：跨二进制替换 2 条记忆字节全等（写穿 DB 的活体证据）。

## 计数对账

- 基线 203 / 207；净增 14 = TODO-70（magic_write_semantics +4、magic.rs +3）
  + TODO-68（magic_mkdir_guards +7、db_vector +1、libsql_vector_semantics −1：
  k0 子进程探针对 → 进程内单测）。
- 收尾 `cargo test --workspace` = **217 passed / 0 failed / 0 ignored**；
  `--features embed-fastembed` = **221 passed**。零回归。
- clippy `--all-targets` 零警告（含 `magic_mkdir_guards` bool_comparison 修复）。
- `scripts/e2e.sh --require-mount 21111` exit 0（6 facets + 重启持久化 +
  auth + export 强制）；TODO-69 phase A/B 全绿。
- 文件卫生：新文件 ≤400 行（magic_mkdir_guards 约 170），dispatcher.rs
  约 577 行 ≤800。

## 遗留与后续

- **ANN 索引闲置**：`memories_vec_idx` 仍存在但 search 不再消费它（改暴力
  余弦）。单 agent 分区规模下无性能问题；若未来单 agent 记忆量大到需要
  ANN，应找"分区内 top-k"的上游能力而非恢复全局后过滤。
- **fastembed 测试依赖离线缓存**：`HF_HOME` + `HF_HUB_OFFLINE=1` 缺失时
  recall 套件会在网络请求上挂起（本轮实测）；CI 化时需固化该环境变量。
- **重启脚本卫生**：`tmp/regress69_*.sh` 属一次性取证脚本（tmp/ 不入库），
  `/proc/exe` 匹配与 pipefail-safe grep 的教训已记入 TODO-69 文档。
