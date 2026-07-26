# TODO-16 只读端点返回 ROFS

## 目标
对 `/search`、`/where` 下文件的 `write`/`setattr` 返回 `NFS3ERR_ROFS`，而非静默 Ok。

## 核心实现
- `dispatcher::write`：`CAT_SEARCH | CAT_WHERE => NFS3ERR_ROFS`。
- `dispatcher::setattr`：同上；`/index` 仍接受（忽略）setattr 以兼容写前 chmod/truncate。

## 验收手段
`tests/magic_write_semantics.rs`：`read_only_views_reject_write_and_setattr`（search + where 各测 write/setattr）。
`tests/anyhome_magic.rs`：`writing_to_search_is_rejected_as_read_only`（原"静默成功"测试改写）。

## 状态
done

## 证据
4 处 `err as u32 == NFS3ERR_ROFS as u32` 断言通过。

## 依赖
TODO-00。
