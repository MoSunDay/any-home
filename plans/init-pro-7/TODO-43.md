# TODO-43 · `/where` 畸形 FTS 查询可读化

**状态**：done

## 目标
区分「畸形 FTS 查询」与「无结果」，不再让前者表现为不透明 `IO` 错误。

## 变更
- `db/mod.rs`：`where_search` 返回 `WhereOutcome` 枚举——`Rows(Vec<(key,text)>)` 或
  `InvalidFts(cond)`；按 SQLite 错误消息分类畸形 FTS5 MATCH（TODO-40 实测：语法错误 /
  未闭合字符串 / 未知 special query 均为 `SqliteFailure`）。
- 参数绑定不变——注入安全性保持。
- `dispatcher.rs`：`InvalidFts` 渲染为文件首行 `# error: invalid FTS query: <cond>\n`
  （NFS READ 没有 errno 通道，只能以文本承载）；合法查询输出与之前逐字节一致。
