# init-pro-10 · 查漏补缺 + 本机部署 + 依赖仓库测试移植（TODO-56..61 全部完成）

## Goal

在**不动任何对外语义契约**（`MemoryDb`/`MagicRegistry` 公共签名、`fs.db.*` 直连、
magic-path 行为、写路径红线）的前提下：① 修补 init-pro-9 遗留的 1 处文档
off-by-one，补部署产物 gitignore；② 首次本机真部署（release 常驻 +
`--require-mount` 全量 live e2e）；③ 把内嵌依赖仓库（nfsserve/libsql）的**可移植
测试**翻译到 anyhome 公共 API 上锁死上游语义，计数对账入 README/changelog。

## 契约红线（本迭代不触碰）

1. 每个 WRITE RPC 写穿 DB（upsert + commit fsync）——继承 init-pro-8。
2. 每个 WRITE RPC 对当次全文做一次 embed——继承 init-pro-8。
3. `MagicRegistry`/`MemoryDb` 公共签名与 `fs.db.*` 直连路径不变。
src/ 本轮**零改动**；红线由既有测试 + 26 个新移植测试共同看护。

## 关键技术决策

| 决策 | 结论 | 证据 |
|---|---|---|
| D1 移植范围 | 只移植"单连接纯 SQL 语义"子集：libsql server/replication/encryption/CLI 与绝大多数 TCL harness 不可移植；nfsserve 上游测试总量 = identity.rs 内联 2 例。不跑上游原套件，只把场景翻译到 anyhome 公共 API | TODO-57/58/59 |
| D2 差异处理策略 | 移植暴露的差异一律**按实测现状钉死 + 注释出处**，不改 src/：k=0 SIGABRT（子进程探针）、search 输出 key 序（JOIN planner）、agent 过滤后置、NEAR/no-such-column Err 传播、rebuild 不自动触发、fuzz 非空期望不作门禁（读路径非确定） | TODO-58/59 |
| D3 k=0 abort 记录方式 | 不在 anyhome 侧拦截（会改 search 语义），用 env-guarded 子进程探针钉死 SIGABRT；上游修复后探针响亮失败提醒摘除 | TODO-58 |
| D4 fuzz 随机源 | 内联 xorshift32 固定 seed，不引 rand crate（确定性 + 零新依赖） | TODO-58 |
| D5 部署形态 | release + dummy embedder（离线可复现）；e2e 独立端口 21111 避开常驻 11111；部署产物由新 gitignore 条目兜底 | TODO-56/60 |

## 状态总表

| TODO | 内容 | 状态 |
|---|---|---|
| 56 | off-by-one 修补 + gitignore 部署产物 | done（commit 672972f） |
| 57 | nfsserve identity 契约移植 2 例 | done（commit 9926f18） |
| 58 | libsql 向量语义移植 12 例 | done（commit 2bd9483 + 评审后断言校准） |
| 59 | libsql FTS5 语义移植 12 例 | done（commit 227c461 + a4c568e clippy 二修） |
| 60 | 本机部署 + live e2e（无代码 commit） | done |
| 61 | 文档三件套 + 计数对账 | done（本目录 + changelog + README） |
