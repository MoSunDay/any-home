# TODO-17 getattr 真实大小

## 目标
`/index` 文件 `getattr`/`readdir` 报告真实内容字节数，而非固定 1 MiB。

## 核心实现
- `magic_attr_sized(id, size)`（目录恒 4096；文件取传入值）；`magic_attr` 退化为虚拟默认值薄封装。
- `index_attr(id)`：`get_text` 取长度 → `magic_attr_sized`；`getattr`/`setattr` 复用。
- `MemoryDb::list_with_size()`：`SELECT key, length(CAST(text AS BLOB))`（字节长度，非字符数）。
- `readdir`（/index）用 `list_with_size` 一次性给准确 DirEntry 属性。
- `/search`、`/where` 维持 `MAGIC_FILE_SIZE` 虚拟上限（动态内容，避免每次 getattr 执行检索）。

## 验收手段
`index_getattr_and_readdir_report_real_byte_size`（getattr + readdir size == 19B）；`search_getattr_keeps_virtual_size`。

## 状态
done

## 证据
`attr.size == text.len()`；readdir entry 同。

## 依赖
TODO-00。
