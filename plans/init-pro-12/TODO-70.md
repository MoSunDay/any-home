# TODO-70 · 缺陷 A/B 修复 + 截断语义（getattr 真实尺寸 / O_TRUNC / remove 告警）

## 缺陷 A：查询文件虚拟尺寸

SEARCH/WHERE（及 INDEX）getattr 返回虚拟 `MAGIC_FILE_SIZE`（1<<20）。NFSv3
客户端按 attr size 决定读窗口并对短读 NUL 补齐（RFC 1813），于是
`wc -c`/`cmp`/`stat` 全部被 ~1 MiB NUL 尾迹污染。

**修法**：查询文件 getattr 返回**本次实际渲染内容的字节数**（空结果 = 0）。
渲染按读生成，尺寸随之真实。

## 缺陷 B：覆写残留旧尾

长文本被短文本覆写后，旧尾部字节仍可读出（替换路径未清 stale tail）。

**修法**：CAT_INDEX 的 WRITE 采用整体替换 + `magic::truncate_buffer`：先按新
长度截断缓冲再写内容，grow 场景为 no-op，替换后绝无旧尾残留。

## O_TRUNC / SETATTR(size)

此前截断不落 DB。现在：

- `setattr(size)` 与 open O_TRUNC 均走 `truncate_buffer` → 写穿 DB（保持
  每 WRITE 单次写、单次 embed 的红线；截断后文本变更才重新 embed）。
- `magic.rs` 新增 3 例单元测试钉死 helper 边界；`magic_write_semantics.rs`
  新增 4 例（缩短/清零/超 EOF no-op/重启持久）。

## remove 告警

CAT_INDEX remove 失败原先静默吞错 → 改为日志告警（可观测性，不改返回语义）。

## 测试翻转（红线内）

- `search_getattr_keeps_virtual_size`（钉死虚拟 1MiB）→ 翻转为
  `search_and_where_getattr_report_real_size`（真实尺寸断言），注释记录旧
  语义与翻转原因。
- `anyhome_magic.rs` 空查询 attr 断言 `MAGIC_FILE_SIZE` → `0`。
