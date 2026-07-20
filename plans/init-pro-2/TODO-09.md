# TODO-09 `/where` FTS5 全文过滤

## 目标
`/where/<词>` 做全文检索；`/where/key:<前缀>` 做 key 前缀过滤。
结果格式 `key\ttext`。

## 核心实现
- FTS5 虚拟表 `memories_fts`（contentless 或 content=`memories` 外部内容表）。
- 3 个触发器自动同步 memories -> memories_fts：
  - `ai`（AFTER INSERT）
  - `ad`（AFTER DELETE）
  - `au`（AFTER UPDATE）
- `where_search()` 解析 `key:` 前缀分支走 LIKE（key 前缀过滤），其余走 FTS5 MATCH。
- 删除后 FTS5 自动清理（触发器，无需手动维护）。
- `get_text()` / `list_keys()` 辅助查询。

## 验收手段
`tests/where_fts.rs`——5 个硬 gate：
1. 单词匹配
2. 跨文档共享词匹配
3. key 前缀过滤（`key:<前缀>`）
4. 删除后不再命中
5. 空条件返回空

## 状态
done

## 证据
- where_fts 5/5 passed。

## 卡点
无。

## 依赖
TODO-08。
