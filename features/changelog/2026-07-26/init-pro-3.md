# init-pro-3：魔法路径正确性与健壮性（TODO-14..20）

## 背景

init-pro / init-pro-2 已交付可挂载的 NFSv3 记忆服务 MVP（35/39 测试绿）。深入代码后发现
**核心交互路径（魔法路径）存在多处正确性缺陷**，会导致隐蔽的数据损坏或语义不自洽：

- `write` **忽略 offset**：`echo`/`cp` 写大文本被 NFS 客户端拆成多个 WRITE RPC 时互相覆盖，只留最后一段。
- `create_exclusive` **不校验独占**：对已存在文件仍返回 Ok。
- 对只读端点 `/search`、`/where` 的 `write`/`setattr` **静默成功空操作**。
- `getattr` 永远**虚报 1 MiB**，`ls -l`/`stat` 字节数错误。
- 魔法 fileid→name 映射是**纯内存态**，重启后旧句柄 `read`/`write` 返回 `NOENT`。
- `write` 用 `String::from_utf8_lossy` **破坏二进制**。

本迭代专注把已交付能力做对、做稳，**不引入新功能**。多 agent 命名空间隔离作为后续方向。

## 变更

### TODO-14 write 支持 offset（`dispatcher.rs` + `magic.rs`）
- `MagicRegistry` 新增进程内**写入缓冲区**（`HashMap<fileid3, Vec<u8>>`）与
  `write_into(id, offset, data)`（按 offset 拼接、零填空洞）、`seed_buffer`、`has_buffer`。
- `dispatcher::write` 改为：首次写时按 DB 既有内容 seed 缓冲 → 按当前 offset splice → 把**已拼接全文** upsert 落库（牺牲写入放大换正确性，NFS 无可靠 close 回调）。
- 不再用 `String::from_utf8_lossy`（见 TODO-19）。

### TODO-15 create_exclusive 独占语义（`dispatcher.rs` + `db/mod.rs`）
- `MemoryDb::exists(key)`：`SELECT 1 ... LIMIT 1`。
- `create_exclusive`：`/index` 查 DB，`/search`/`/where` 查内存注册表；已存在返回 `NFS3ERR_EXIST`。

### TODO-16 只读端点显式错误（`dispatcher.rs`）
- 对 `/search`、`/where` 文件的 `write`/`setattr` 返回 `NFS3ERR_ROFS`，而非静默 Ok。
- `/index` 仍接受（忽略）setattr 以兼容写前 chmod/truncate 的客户端。

### TODO-17 getattr 真实大小（`dispatcher.rs` + `magic.rs` + `db/mod.rs`）
- `magic_attr_sized(id, size)`：可指定 size 的属性构造（目录恒 4096，文件取传入值）；`magic_attr` 退化为虚拟默认值的薄封装。
- `dispatcher::getattr` / `setattr` 对 `/index` 调用 `index_attr(id)` 返回**真实字节数**。
- `MemoryDb::list_with_size()`：`SELECT key, length(CAST(text AS BLOB))`（字节长度，非字符数），供 `readdir` 一次性给出准确 DirEntry 属性。
- `/search`、`/where` 内容随查询动态生成，维持虚拟上限值（`MAGIC_FILE_SIZE`，文档化为"慷慨上限"），避免每次 getattr 都执行检索。

### TODO-18 句柄跨重启存活（`dispatcher.rs`）
- 新增 `index_name_for(id)`：内存映射命中即返回；未命中则扫描 DB keys 用确定性 fileid（`FNV-1a(name)`）反查并缓存。
- `read`/`getattr`/`write` 的 `/index` 路径改用 `index_name_for`，旧句柄重启后仍可读。
- `/search`、`/where` 的 name 即 query、无 DB 后端，重启后由客户端重新 `lookup` 自然恢复（标准 NFS 重连语义，已文档化）。

### TODO-19 write 二进制安全（`dispatcher.rs`）
- DB `text` 列要求 UTF-8：非 UTF-8 写入显式返回 `NFS3ERR_INVAL`，而非 `from_utf8_lossy` 静默替换。
- 已知限制：若多段 WRITE 恰好在一个多字节字符中间断开，中间态校验会失败（记忆文本通常一次性写入，属可接受边界）。

### TODO-20 回归测试 + 文档
- 新增 `tests/magic_write_semantics.rs`（10 个用例）。
- 更新 `tests/anyhome_magic.rs`：`writing_to_search_*` 由"静默成功"改为断言 `ROFS`。
- 修复 `scripts/bootstrap.sh`：原 pin 的"memory baseline"覆盖提交从未发布到上游，改为 pin 上游 HEAD（= base），并加 `--depth 1`。
- 更新 README「状态」与本 changelog。

## 验收

| 验收点 | 测试 | 文件 |
|---|---|---|
| 多段 WRITE 按 offset 拼接、不丢数据 | `index_write_composes_many_chunks`（2000B 分 200 段） | tests/magic_write_semantics.rs |
| 追加写拼接正确 | `index_write_appends_at_offset` / `index_write_composes_with_existing_content` | 同上 |
| exclusive 冲突返回 EXIST（index + search） | `index_create_exclusive_conflicts_with_existing` / `search_create_exclusive_conflicts_after_lookup` | 同上 |
| 只读端点 write/setattr 返回 ROFS | `read_only_views_reject_write_and_setattr` | 同上 |
| `/index` getattr/readdir size == 内容字节数 | `index_getattr_and_readdir_report_real_byte_size` | 同上 |
| `/search` 维持虚拟 size | `search_getattr_keeps_virtual_size` | 同上 |
| 重启后旧句柄可读 + size 正确 | `index_handle_survives_restart` | 同上 |
| 非 UTF-8 写入返回 INVAL 且不落库 | `index_write_rejects_invalid_utf8` | 同上 |

- 全量回归：`cargo test --workspace` → **45 passed; 0 failed**（默认）
- fastembed 回归：`cargo test --workspace --features embed-fastembed` → **49 passed; 0 failed**
- clippy：`cargo clippy --workspace --all-targets -- -D warnings` → 零警告
- 构建：`cargo build --workspace`（默认）✓ + `--features embed-fastembed` ✓（需 `pkg-config`/`libssl-dev`，已就绪）
- 行数：`dispatcher.rs` 461 ≤ 800；`magic.rs` 198、`db/mod.rs` 381 ≤ 400

## Impact Surface

- **对用户可感知**：大文本写入不再丢数据；`ls -l`/`stat` `/index` 文件显示真实字节数；向 `/search`、`/where` 写入得到明确的只读错误；重启后历史记忆可继续读写。
- **不影响**：NFS 协议边界、RealFs 分流、向量 KNN / FTS5 检索路径、持久化 schema（无 schema 变更）。
- **回归面**：魔法路径写/读/属性由 `anyhome_magic` + `magic_write_semantics` 双重覆盖；DB 层仅新增只读查询方法。

## Related Docs

- [plans/init-pro-3/index.md](../../../plans/init-pro-3/index.md)
- [既有 changelog：init-pro-2](../2026-07-20/init-pro-2.md)
