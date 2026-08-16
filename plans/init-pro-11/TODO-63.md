# TODO-63 · libsql FTS 查询语法 + unicode61 移植（31 例）

**状态**：done（commit c002c7d）

## 范围决策
上游 fts5aa（§2.3/§12/§13）、fts5prefix（§1.1/§3.1）、fts5ac（§3–§9）的纯查询
语义场景，以及 fts5unicode（§1.x/§2.x）、fts5umlaut（§1.x）的分词场景，经
`MemoryDb` 公共 API 可移植。不可移植面在文件头注释在案：影子表/DDL/integrity/
pgsz/损坏/事务内部、prefix= 索引选项、highlight()/snippet()、checksum、ascii
tokenizer、separators/tokenchars/remove_diacritics=2 配置、detail=none/column、
bm25 权重值——固定 schema 下无 API 面。

## 变更
1. `tests/libsql_fts_query_syntax.rs`（290 行，21 例）：AND/OR/NOT、隐式 AND、
   括号优先级、短语、NEAR/NEAR-k、前缀 `*`、列过滤 `text:` / `{text}:`。
2. `tests/libsql_fts_unicode.rs`（227 行，10 例）：大小写折叠（含西里尔双向）、
   变音符（umlaut res1 矩阵 0,1,1,0,1,1 精确复现；预组合 ộ U+1ED9 在默认
   remove_diacritics=1 下**不**折叠——需 mode 2；组合记号恒被跳过）、分隔符
   （下划线拆分；foo-bar）、token 内数字（x86/abc123 为单 token）。

## 实测钉死的不对称（注释在案）
1. `NEAR(a b, k)` 距离 = 中间隔开的最大 token 数。
2. `near(...)` 小写 → InvalidFts；中缀 `a NEAR b` 按三词隐式 AND 解析。
3. 特殊查询 `* reads` 经 memories_fts⋈memories JOIN 得零行；`*stuff`/`*` →
   InvalidFts。
4. 未加引号 `foo-bar` → 否定 colset 错误 "no such column: bar"，不在
   InvalidFts 白名单 → 硬 Err（加引号 `"foo-bar"` 按短语工作）。
5. `baz.qux` 查询 → InvalidFts，而含点文本分词正常（lexer/tokenizer 不对称）。

## 验证
两文件各连续 20 次全绿（21/21、10/10）；见 index.md 计数对账。
