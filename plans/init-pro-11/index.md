# init-pro-11 · 查漏补缺 + FTS 语法/unicode/深度/hostile-text 移植 + 再部署（TODO-62..66 全部完成）

## Goal

在**不动任何对外语义契约**（`MemoryDb`/`MagicRegistry` 公共签名、每个 WRITE RPC
写穿 DB、每次写入单次 embed、`fs.db.*` 直连路径全部不变）的前提下：① 补齐
init-pro-10 遗留的契约空洞——`build_auth_policy` 合并逻辑零测试，把它纯逻辑平移
进 `src/auth.rs` 并补 7 例单元测试，顺带修 e2e 脆弱断言与 dbops 静默 IO；② 继续
把 libsql FTS5 上游可移植场景（查询语法 / unicode61 / 表达式深度）与 hostile-text
roundtrip 翻译到 anyhome 公共 API 上钉死；③ 换用新二进制再部署并全量 live e2e；
④ 文档三件套 + 计数对账。

## 摸底结论

1. **auth 合并逻辑零测试**：`build_auth_policy`（flag ∩ file 去重、并集合并、
   空回退 Open、坏文件报错含路径）是 soft-auth 的唯一入口，却从未被任何测试覆盖
   ——契约红线看护缺口。且它位于 `main.rs`，无法被 lib 测试触达。
2. **FTS 查询语法面未移植**：init-pro-10 只移植了 fts5content/rank/rebuild/ea
   四套；上游 fts5aa（§2.3/§12/§13 布尔/短语/NEAR）、fts5prefix（§1.1/§3.1）、
   fts5ac（§3–§9）、fts5unicode（§1.x/§2.x）、fts5umlaut（§1.x）的纯查询语义场景
   在 anyhome 侧无钉死。
3. **表达式深度未钉死**：fts5expr/fts5limits 的深度边界（括号嵌套、NOT 链、
   AND/OR 扁平化、长短语）从未在捆绑 libSQL 上实测；上游只测过 260 项链。
4. **hostile-text 无 roundtrip 门禁**：引号/反斜杠/控制字符/emoji/NUL/超大文本
   /超长 key 经公共 upsert/get_text 是否字节全等，此前无测试。
5. **e2e 两处卫生问题**：`--export` MOUNT 强制只有正例没有负例（挂错 export 名
   应被拒）；5 处 grep 模式用字面 TAB，编辑器/复制粘贴易碎。
6. **dbops 6 处静默 `NFS3ERR_IO`**：embed/query 失败静默吞错，日志无痕迹。

## 契约红线（继承，不触碰）

1. `MemoryDb`/`MagicRegistry` 公共签名不变。
2. 每个 WRITE RPC 写穿 DB（upsert + commit fsync）——继承 init-pro-8。
3. 每个 WRITE RPC 对当次全文做一次 embed——继承 init-pro-8。
4. `fs.db.*` 直连路径全部不变。

src/ 本轮改动**仅限**：`build_auth_policy` 纯逻辑平移（main.rs → auth.rs，逐字节
相同：tracing 消息、`--agent-creds {path:?}: {e}` 错误包装均不变）+ dbops 6 处
补 `tracing::warn`（返回值与控制流零变化）。红线由既有 149 用例 + 54 个新用例
共同看护。

## 关键技术决策

| 决策 | 结论 | 证据 |
|---|---|---|
| D1 export MOUNT 负例的钉死面 | 选 nfsserve `with_export_name` 拒绝错误挂载名（MNT3ERR_NOENT）：强制逻辑在只读 nfsserve 依赖内，anyhome 无单元面可测，只记录不造桩 | TODO-62 |
| D2 差异处理策略 | 移植暴露的差异一律**按实测现状钉死 + 注释出处**，不改 src/：`near(...)` 小写 → InvalidFts 而中缀 NEAR → 三词隐式 AND；`* reads` 特殊查询零行；`foo-bar` 未加引号 → 否定 colset Err（非白名单）；`baz.qux` → InvalidFts | TODO-63 |
| D3 深度边界 spike-then-pin | 先探后钉：括号嵌套 ≤97 Rows / ≥98 InvalidFts（因 lemon YYSTACKDEPTH=100，非 SQLITE_MAX_EXPR_DEPTH）；NOT 链 ≤255 Rows / ≥256 **Err**（消息无 `fts5:` 前缀故被传播）；全深度无 abort，无需子进程探针（与向量 k=0 不同） | TODO-64 |
| D4 hostile-text 钉死策略 | NUL 字节写入成功但**首 NUL 截断**按现状钉死；1 MiB 文本 / 64 KiB key roundtrip 钉为常设保证（8 MiB / 1 MiB key spike 也过，取保守值） | TODO-64 |
| D5 部署形态 | 沿用 init-pro-10：release + dummy embedder（离线可复现），e2e 独立端口 21111；换二进制后旧记忆跨重启存活 = write-through 耐久证据 | TODO-65 |
| D6 magic 文件读 NUL 填充 | 读取魔法文件会补 NUL 至 MAGIC_FILE_SIZE（1<<20）——magic.rs 既有行为，只记录不修，不影响 grep 型断言；候选未来 TODO | TODO-65 |
| D7 记录不测 | fastembed-missing bail 路径需 feature-missing 构建矩阵；libsql Rust 侧 29/31 不可移植（replication/encryption/raw-Connection 面）——均只记录 | TODO-62/63 |

## 状态总表

| TODO | 内容 | 状态 |
|---|---|---|
| 62 | 查漏补缺：build_auth_policy 平移 + 7 例单元测试 + e2e export 强制/TAB 卫生 + dbops 补日志 | done（commit db862bf） |
| 63 | FTS 查询语法 + unicode61 移植 31 例（21+10） | done（commit c002c7d） |
| 64 | 表达式深度 + hostile-text 钉死 16 例（7+9） | done（commit 17711f3） |
| 65 | 本机再部署 + live e2e（运维步骤，无代码 commit） | done |
| 66 | 文档三件套 + 计数对账 | done（本目录 + changelog + README） |

## 计数对账

- 基线（init-pro-10 收尾）：`cargo test --workspace` **149 passed**；
  `--features embed-fastembed` **153 passed**。
- 新增 54 = auth 单元 7 + 查询语法 21 + unicode 10 + 表达式深度 7 + hostile-text 9。
- 收尾：workspace **203 passed**（149+54）；fastembed **207 passed**（153+54，
  同一批 +54）。
- 集成测试文件 16 → 20（新增 libsql_fts_query_syntax / libsql_fts_unicode /
  libsql_fts_expr_depth / hostile_text_roundtrip）；新文件均 ≤400 行
  （290/227/189/203），src 文件均 ≤800 行；无新增 `#[ignore]`。
- 验收复跑：workspace 连续 3 次全绿；每个新测试文件连续 20 次全绿；clippy
  `-D warnings` 双 feature 组合零警告。
