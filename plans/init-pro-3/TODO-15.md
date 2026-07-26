# TODO-15 create_exclusive 独占语义

## 目标
魔法文件 `create_exclusive` 校验目标是否已存在，已存在返回 `NFS3ERR_EXIST`。

## 核心实现
- `MemoryDb::exists(key)`：`SELECT 1 FROM memories WHERE key = ? LIMIT 1`。
- `create_exclusive`：`/index` 查 DB（权威）；`/search`/`/where` 查内存注册表；命中即 `NFS3ERR_EXIST`。

## 验收手段
`tests/magic_write_semantics.rs`：`index_create_exclusive_conflicts_with_existing`、`search_create_exclusive_conflicts_after_lookup`；新 key 仍成功。

## 状态
done

## 证据
两次断言 `err as u32 == NFS3ERR_EXIST as u32`。

## 依赖
TODO-00。
