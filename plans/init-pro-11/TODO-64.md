# TODO-64 · 表达式深度 + hostile-text 钉死（16 例）

**状态**：done（commit 17711f3）

## 范围决策
fts5expr/fts5limits 的深度边界在捆绑 libSQL 上**先探后钉**（spike-then-pin）：
实测确定边界值再写死为断言，不照抄上游阈值。hostile-text 经公共
upsert/get_text 做字节全等 roundtrip。

## 变更
1. `tests/libsql_fts_expr_depth.rs`（189 行，7 例）：
   - 括号嵌套 ≤97 → Rows，≥98 → InvalidFts（`fts5: parser stack overflow`）；
     根因 = lemon `YYSTACKDEPTH=100`（fts5parse.y），**非** SQLITE_MAX_EXPR_DEPTH。
   - NOT 链 ≤255 → Rows，≥256 → **Err**（非 InvalidFts）——消息
     `fts5 expression tree is too large (maximum depth 256)` 缺 `fts5:` 前缀，
     故 classify_fts_error 放行传播；SQLITE_FTS5_MAX_EXPR_DEPTH=256。
   - AND/OR 链扁平化（10000 项链正常；上游只测过 260）。
   - 嵌套 NEAR → InvalidFts；20000-token 短语正常。
   - **全深度无 abort**（与向量 k=0 不同，无需子进程探针）。
2. `tests/hostile_text_roundtrip.rs`（203 行，9 例）：引号、反斜杠、\n/\r\n、TAB、
   %/=/_、emoji/RTL/组合字符（**不做归一化**：`e`+U+0301 与 `é` 存为不同）经
   公共 upsert/get_text 字节全等。钉死探针：
   - **NUL 字节写入成功但首 NUL 截断**（text 与 key 查询均如此）。
   - **1 MiB 文本精确 roundtrip**（8 MiB 探针亦 OK，无可观察上限）。
   - **64 KiB key roundtrip** 且存活于 list_keys（1 MiB key spike 亦通过；
     64 KiB 钉为常设保证）。

## 验证
两文件各连续 20 次全绿（7/7、9/9）；见 index.md 计数对账。
