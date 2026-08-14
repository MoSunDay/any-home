# TODO-40 · 现状 spike：大小 oracle + FTS 错误实测

**状态**：done

## 目标
核实 init-pro-5 鉴权的实际拦截面，确认「泄漏句柄已收敛」语义是否存在漏洞；实测畸形 FTS5
MATCH 的真实行为，验证旧代码注释「bad FTS queries -> no rows」。

## 结论（关键前提更正）
- **大小 oracle 证实**：`lookup` 在**已授权**调用方名下向进程级 MagicRegistry 注册
  `MagicEntry`；fileid 可离线推导（`"{agent}\0{key}"` 的 FNV-1a + CAT_INDEX 偏移）——
  持有/猜中 fileid 的**未授权**调用方 `getattr` 即返回真实存储字节数。
- **旧注释错误**：畸形 FTS5 MATCH 并非「无行返回」，而是在 fetch 时抛 libsql
  `SqliteFailure`，错误消息形如 `fts5: syntax error near ...`、`unterminated string`、
  `unknown special query: ...`（实测验证）。

## 产出
- 修正后的鉴权矩阵（见 [index.md](index.md)），驱动 TODO-41 的最小拦截面设计；
  同时确认 TODO-43 需按错误消息分类而非吞错。
