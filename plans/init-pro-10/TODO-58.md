# TODO-58 · libsql 向量语义移植（12 例）

**状态**：done（commit 2bd9483）

## 范围决策
`libsql_vector.test`/`libsql_vector_index.test` 中**单连接纯 SQL** 的 KNN 场景可
移植；server/replication/encryption/CLI、md5 钉死的 v2 index 元数据用例无
`MemoryDb` 公共 API 对应面，不移植（reopen 不变性替代）。`vector_fuzz_test`
（integration_tests.rs）整体移植，rand → 内联 xorshift32（seed 0x5EED_BA5E）。

## 变更
`tests/libsql_vector_semantics.rs`（332 行，12 例）：backfill 序（按 k 钉余弦
top-k 选择 + key 序输出）、upsert 覆盖翻转排名 + 旧向量消失、delete 从 KNN 消失、
空表 search、等距 tie-break 同构建确定性、k>行数全返、维度不匹配 Err 含
`dimensions are different`、agent 过滤后置 <k 行、文件 DB reopen 不变、fuzz 200
次 + alive-set 不变量、k=0 abort（子进程探针 + env-guarded 内探针）。

## 实测钉死的三处语义（注释在案）
1. `search(.., 0)` → 捆绑 libSQL 向量索引堆损坏 SIGABRT（子进程钉死）。
2. 输出按 key 序（JOIN 被 `sqlite_autoindex_memories_1` 驱动，EXPLAIN 实证）。
3. agent 过滤在全局 top-k 之后（单 agent 可能 <k/0 行）。
