# init-pro-11：查漏补缺 + FTS 语法/unicode/深度/hostile-text 移植 + 再部署（TODO-62..66）

## 背景

init-pro-10 把 nfsserve/libsql 的可移植测试搬进了 anyhome，但摸底暴露四处空洞：
① soft-auth 的唯一入口 `build_auth_policy`（flag/file 合并、去重、空回退、坏文件
报错）**零测试**，且位于 `main.rs` 无法被 lib 测试触达；② FTS5 只移植了
content/rank/rebuild/ea 四套，上游 fts5aa/fts5prefix/fts5ac/fts5unicode/
fts5umlaut 的查询语法与分词场景无钉死；③ 表达式深度边界（fts5expr/fts5limits）
从未实测；④ hostile-text（引号/控制字符/emoji/NUL/超大文本/超长 key）无 roundtrip
门禁。另有两处卫生项：e2e 缺 export 强制负例且 5 处字面 TAB grep 易碎、dbops 6 处
静默吞错。取证基线：`cargo test --workspace` 149 passed / `--features
embed-fastembed` 153 passed。

## 变更

1. **TODO-62 查漏补缺**：
   - `build_auth_policy` **纯逻辑平移** main.rs → `src/auth.rs`（逐字节相同：
     tracing 消息与 `--agent-creds {path:?}: {e}` 错误包装不变），main.rs 改调
     `anyhome::auth::build_auth_policy(&cli)?`（125 → 101 行）；补 7 例单元测试：
     flag-only Enforced、creds-file-only、flag∩file 重复绑定集合语义去重、并集
     合并、空 → Open 回退、缺文件 Err 含路径、坏文件（uid 0 被拒）Err 含路径。
     **补上契约红线看护缺口**。
   - `scripts/e2e.sh`：`--export` MOUNT 强制补**负例**（挂错误 export 名被拒，
     MNT3ERR_NOENT）——强制逻辑在只读 nfsserve 依赖内、anyhome 无单元面，选
     `with_export_name` live 钉死（决策在案）；5 处字面 TAB grep 改 `${T}`。
   - `src/dispatcher/dbops.rs`：6 处静默 `NFS3ERR_IO` 补
     `tracing::warn!(op=..., error=%e)`，返回值与控制流零变化。
2. **TODO-63 FTS 查询语法 + unicode61 移植（31 例）**：
   `tests/libsql_fts_query_syntax.rs`（290 行，21 例，来源 fts5aa §2.3/§12/§13 +
   fts5prefix §1.1/§3.1 + fts5ac §3–§9）：AND/OR/NOT、隐式 AND、括号优先级、
   短语、NEAR/NEAR-k、前缀 `*`、列过滤。`tests/libsql_fts_unicode.rs`（227 行，
   10 例，来源 fts5unicode §1.x/§2.x + fts5umlaut §1.x）：大小写折叠（含西里尔
   双向）、变音符（umlaut res1 矩阵 0,1,1,0,1,1 精确复现）、分隔符、token 内
   数字。不可移植面（影子表/DDL/prefix= 选项/highlight/snippet/ascii tokenizer/
   remove_diacritics=2/detail=none/bm25 权重等）文件头注释在案。
3. **TODO-64 表达式深度 + hostile-text 钉死（16 例）**：
   `tests/libsql_fts_expr_depth.rs`（189 行，7 例，spike-then-pin 对照
   fts5expr/fts5limits）与 `tests/hostile_text_roundtrip.rs`（203 行，9 例，经
   公共 upsert/get_text 字节全等）。细节见下节。
4. **TODO-65 本机再部署**：停 init-pro-10 实例（pid 20630）→ release 重建 →
   setsid 常驻 127.0.0.1:11111（dummy embedder，dim 64；启动日志
   `anyhome::auth` policy assembly = Open 证实平移函数就位）→
   `scripts/e2e.sh --require-mount 21111` exit 0（真实 mount，含错误 export
   拒绝负例）→ 手工冒烟全 OK，init-pro-10 记忆跨二进制替换存活（write-through
   耐久证据）；无代码 commit。
5. **TODO-66 文档三件套**：本 changelog + `plans/init-pro-11/`（index +
   TODO-62..66）+ README 双语计数 149/153 → 203/207、结构行 16 → 20 文件。

## 实测钉死的关键语义

- **深度边界两套机制**：括号嵌套 ≤97 Rows / ≥98 InvalidFts（`fts5: parser
  stack overflow`，根因是 lemon `YYSTACKDEPTH=100`，非 SQLITE_MAX_EXPR_DEPTH）；
  NOT 链 ≤255 Rows / ≥256 **硬 Err**（消息 `fts5 expression tree is too large
  (maximum depth 256)` 缺 `fts5:` 前缀，不被 classify_fts_error 收编而传播）。
  AND/OR 链扁平化（10000 项链正常，上游只测过 260）；**全深度无 abort**——与
  向量 k=0 堆损坏不同，无需子进程探针。
- **查询语法不对称**：`near(...)` 小写 → InvalidFts，中缀 `a NEAR b` 按三词隐式
  AND 解析；特殊查询 `* reads` 经 memories_fts⋈memories JOIN 得零行而
  `*stuff`/`*` → InvalidFts；未加引号 `foo-bar` → 否定 colset 错误
  "no such column: bar"（不在白名单 → 硬 Err，加引号才按短语工作）；`baz.qux`
  查询 → InvalidFts 而含点文本分词正常（lexer/tokenizer 不对称）。
  `NEAR(a b, k)` 距离 = 中间隔开的最大 token 数。
- **unicode61 分词**：预组合 ộ（U+1ED9）在默认 remove_diacritics=1 下**不**折叠
  （需 mode 2），组合记号恒被跳过；umlaut res1 矩阵精确复现 0,1,1,0,1,1；下划线
  拆分 token；x86/abc123 为单 token。
- **hostile-text**：引号/反斜杠/换行/TAB/`%`/`=`/`_`/emoji/RTL 字节全等往返，
  **不做 Unicode 归一化**（`e`+U+0301 与 `é` 存为不同值）；**NUL 字节写入成功但
  首 NUL 截断**（text 与 key 均如此）；**1 MiB 文本**精确 roundtrip（8 MiB 探针
  亦过）；**64 KiB key** roundtrip 且存活 list_keys（1 MiB key spike 亦过，取
  保守值钉为常设保证）。
- **部署语义**：新二进制启动日志即证实 `build_auth_policy` 平移生效；旧实例记忆
  跨二进制替换可读 = 每个 WRITE RPC 写穿 DB 的活体证据。

## 计数对账

- 基线 149 / 153；新增 54 = auth 单元 7 + 查询语法 21 + unicode 10 + 表达式深度
  7 + hostile-text 9。
- 收尾 `cargo test --workspace` = **203 passed / 0 failed / 0 ignored**；
  `--features embed-fastembed` = **207 passed**（同一批 +54）。零回归。
- 验收复跑：workspace 连续 3 次全绿；每个新测试文件连续 20 次全绿；clippy
  `--all-targets -- -D warnings` 双 feature 组合零警告；新文件 ≤400 行
  （290/227/189/203）、src ≤800 行；无新增 `#[ignore]` / 删测试 / 弱断言。
- 集成测试文件 16 → 20；`scripts/e2e.sh --require-mount 21111` exit 0（真实
  mount，非 skip 路径）。

## 遗留与后续

- **magic 文件读 NUL 填充**（已记录不修）：读取魔法文件补 NUL 至
  MAGIC_FILE_SIZE（1<<20），真实内容后带 ~1 MiB NUL 尾迹——magic.rs 既有行为，
  不影响 grep 型断言；候选未来 TODO。
- **记录不测**：fastembed-missing bail 路径（无 feature 时 `--embedder
  fastembed` 报错）需 feature-missing 构建矩阵；libsql Rust 侧 29/31 测试不可
  移植（replication/encryption/raw-Connection 面）——沿 init-pro-10 记录在案。
- **上游依赖**：k=0 SIGABRT 子进程探针（init-pro-10 引入）继续看护；libSQL
  修复后探针响亮失败提醒摘除。
