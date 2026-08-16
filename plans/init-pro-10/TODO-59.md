# TODO-59 · libsql FTS5 语义移植（12 例）

**状态**：done（commit 227c461 + a4c568e clippy 二修）

## 范围决策
四个 TCL 套件的可移植场景：fts5content（§2.6/§9/§3.1）、fts5rank（6.x）、
fts5rebuild（1.5-1.7）、fts5ea（§3）。其余 fts5* 套件依赖 SQLite TCL harness /
fault-injection / 多连接特性，不可移植。

## 变更
`tests/libsql_fts_semantics.rs`（250 行，12 例）：upsert 更新文本后 MATCH 可见
新内容、delete 后不再命中、词频 rank 序 + 同分稳定、OR 零命中 arm、影子表清空→
损坏→'rebuild' 恢复（`libsql::Builder` 直操文件，沿用 db_persistence 惯例）、
InvalidFts 分类表 7 条、NEAR 非整数距离 Err、no-such-column Err、空 cond 空
Rows、千 token 大文本、unicode61 CJK/emoji 行为。

## 实测钉死的偏差（注释在案）
1. `NEAR(a b, xyz)` → Err（消息前缀不在 classify_fts_error 白名单）。
2. `no such column:` → Err 传播（刻意决策：缺列不属 MATCH 语法错误分类）。
3. external-content 表影子表清空后 open 不自动重建（count 镜像 content 恒非 0），
   恢复需显式 'rebuild' 命令。
4. unicode61：未分词 CJK = 单 token；🔥 为分隔符（MATCH 不可见、get_text 完好）；
   🦀（6.1 后新增）反成可检索 token。
