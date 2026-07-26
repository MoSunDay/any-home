# TODO-18 句柄跨重启存活

## 目标
重启后（内存映射丢失）`/index` 旧句柄仍可 `read`/`getattr`，不再返回 `NOENT`。

## 核心实现
- `index_name_for(id)`：内存命中即返回；未命中则扫描 `list_keys()`，用确定性 fileid（`FNV-1a(name)`）反查匹配 key 并 `register` 缓存。
- `read`/`getattr`/`write` 的 `/index` 路径改用 `index_name_for`。
- `/search`/`/where` 的 name 即 query、无 DB 后端，重启后由客户端重新 `lookup` 自然恢复（标准 NFS 重连语义）。

## 验收手段
`index_handle_survives_restart`：写 → 新 AnyHomeFs（空注册表）→ lookup + read 回原文 + getattr size 正确。

## 状态
done

## 证据
重启后 `attr.size == len` 且读回原文。

## 依赖
TODO-00。
