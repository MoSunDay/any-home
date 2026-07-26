# TODO-19 write 二进制安全

## 目标
去除 `String::from_utf8_lossy` 的静默替换；非 UTF-8 写入显式报错，不破坏数据。

## 核心实现
- DB `text` 列要求 UTF-8：`write` 对已拼接缓冲 `String::from_utf8(full)`，失败返回 `NFS3ERR_INVAL`（在 `run_index` 之前，故不落库）。
- 已知限制：多段 WRITE 恰好在多字节字符中间断开时中间态校验失败（记忆文本通常一次性写入）。

## 验收手段
`index_write_rejects_invalid_utf8`：写 `0xFF` → `NFS3ERR_INVAL`；`get_text("bin")` 为 None。

## 状态
done

## 证据
`err as u32 == NFS3ERR_INVAL as u32` 且 `is_none()`。

## 依赖
TODO-14。
