# TODO-68 · 6 e2e facets + 缺陷 C/E 修复（mkdir 守卫 / scoped KNN）

## 6 个 e2e facets（补齐未覆盖魔法面）

`scripts/e2e.sh` 追加（原 DIAG 诊断块已移除）：

1. `/index/<key>` 回读 + **真实 attr size**（缺陷 A 修复的线上证据）。
2. 覆写 + O_TRUNC 无 stale tail（缺陷 B）。
3. rm 三重检查：NFS 枚举、`/where` 前缀查询、DB 行全部消失。
4. 真实文件生命周期落在底层 FS（非魔法路径）。
5. unicode round-trip：UTF-8 文本 + `/search`、`/where` UTF-8 查询。
6. `/search_k<N>` 上限（全局 + **agent 作用域**）——正是本 facet 触发缺陷 E。

## 缺陷 C：mkdir -p 落穿真实 FS（根因实锤）

**取证**：`RUST_LOG=nfsserve=debug` + strace 双通道确认——GNU `mkdir -p` 使用
chdir 技巧后发**相对** `mkdir("p")` RPC，逐分量**不做 LOOKUP**；而裸
`mkdir`/`touch` 会先 LOOKUP。于是 `mkdir -p agents/botA/index` 的第一分量
`mkdir(ROOT_ID, "agents")` 在 dispatcher 无守卫时落穿到 RealFs：真实
`anyhome-fs/agents` 目录被创建，**shadow** 虚拟 agents 目录（lookup 优先命中
真实条目），后续所有 agent 遍历走 real 分支。此前疑似的"缺陷 D"（agent 面
异常）只是 C 的下游症状，不是独立 bug。

**修法**（`dispatcher.rs`，纯增量）：虚拟保留名对"创建类" RPC 主张
`NFS3ERR_EXIST`——客户端按 NFSv3 语义回退 re-LOOKUP，命中魔法 fileid：

- `reserved_root_name()`：`search`/`index`/`where`/`agents` + `parse_search_k`
  解析出的 `search_k<N>` 全家族。
- MKDIR(ROOT, 保留名) → EXIST；MKDIR(AGENTS_DIR, 任意名) → require_agent 后
  EXIST；MKDIR(CAT_AGENT 目录或规范魔法名) → EXIST，其余 NOTSUPP。
- CREATE/CREATE_EXCLUSIVE/SYMLINK(ROOT, 保留名) → EXIST。
- REMOVE(ROOT, 保留名) → INVAL；RENAME 目标为保留名 → EXIST、源为保留名 → INVAL。
- READDIR 根目录 `retain(!reserved)`：隐藏已被历史落穿 shadow 的保留名条目。

**测试**：`tests/magic_mkdir_guards.rs` 7 例——保留名 mkdir EXIST、真实目录
mkdir 不受影响、agent 子目录 EXIST/NOTSUPP、create/remove/rename/symlink
拦截、readdir 隐藏 shadow 条目、`mkdir -p agents/<id>/index` 全流程落库且
零真实目录。线上复验：冷启动 `mkdir -p agents/botA/index` + 写入 + agent
where/search 全通，`anyhome-fs/` 不生长任何目录。

## 缺陷 E：全局 top-k 后过滤导致 scoped search 饥饿

**症状**（facet 6）：botA 有记忆，`/agents/botA/search_k1` 空返回。

**bisect 复现链**：`tmp/repro_sk10..13.sh` 逐步收窄，v8 变体锁定——botA+botB
各有记忆、botB 的行离查询更近时，botA 的 k=1 搜索必空。

**根因**：旧 SQL 先对**全局** ANN 索引 `vector_top_k('memories_vec_idx',
vector(?), k)` 取 k 行，再 JOIN + `WHERE agent_id=?` 过滤——他 agent 更近的
行吃满全部 k 槽，本 agent 一行都进不来。附带风险：`vector_top_k` 在捆绑
libSQL 上 k=0 触发堆损坏 SIGABRT（init-pro-10 曾以子进程探针钉死）。

**修法**（`db/query.rs`，`MemoryDb` 公共签名不变）：按 agent 分区的**精确
KNN**——

```sql
SELECT key, text FROM memories
WHERE agent_id = ? AND emb IS NOT NULL
ORDER BY vector_distance_cos(emb, vector(?)) ASC, id ASC
LIMIT ?
```

捆绑 libSQL 只注册 `vector_top_k`/`vector_distance_cos`/`vector_distance_l2`
（`sqlite3.c:215980` 函数表），无需 ANN 索引；单 agent 分区暴力余弦在记忆库
规模下精确且足够快；`id ASC` 破平保证确定序。`vector_top_k` 不再被 search
触达，k=0 abort 路径对 anyhome API 不可达。

**测试**：

- `db_vector.rs` +1：`scoped_search_survives_other_agents_nearer_rows`
  （饥饿回归例，直接复刻 bisect v8）。
- `libsql_vector_semantics.rs` 6 处旧钉子按红线翻转（均留注释）：距离序输出
  （backfill/k_larger）、`id ASC` 破平（equidistant）、scoped 不饥饿
  （agent_filter → scoped_search_is_not_starved）、维度错配新措辞
  （`vector_distance: vectors must have the same length: 3 != 2`）、k=0
  子进程 SIGABRT 探针 → 进程内空结果断言。
