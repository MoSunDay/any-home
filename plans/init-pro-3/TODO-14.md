# TODO-14 write 支持 offset

## 目标
魔法文件 `write` 按 offset 拼接，使多段 WRITE（`echo`/`cp` 大文本被拆成多个 RPC）正确累积，不再互相覆盖。

## 核心实现
- `MagicRegistry` 增进程内写入缓冲 `HashMap<fileid3, Vec<u8>>`：`write_into(id, offset, data)` 按 offset splice + 零填空洞；`seed_buffer` / `has_buffer`。
- `dispatcher::write`（CAT_INDEX）：首次写按 DB 既有内容 seed → splice → upsert **已拼接全文**。
- 落库时机：每次 WRITE 即 upsert 全文（无 fsync/commit 钩点时的正确性优先策略；牺牲写入放大）。

## 验收手段
`tests/magic_write_semantics.rs`：`index_write_composes_many_chunks`（2000B 分 200 段，DB 逐字节一致）；`index_write_appends_at_offset`；`index_write_composes_with_existing_content`（seed 后追加）。

## 状态
done

## 证据
`index_write_composes_many_chunks` 断言 `stored.as_bytes() == expected`。

## 依赖
TODO-00。
