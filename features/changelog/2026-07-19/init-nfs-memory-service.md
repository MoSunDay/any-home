Commit: (working-tree, pre-initial-commit)

# 初始实现：可挂载的 NFSv3 记忆服务（/search 向量 KNN + /index 写入即索引）

## 背景

anyhome 此前是纯文档仓库（仅 README + 计划）。本次按 `plans/init-pro` 把它落地为可工作的
MVP：一个 localhost 上的 NFSv3 服务，agent 用最朴素的文件操作（`cat`/`echo`/`ls`）即可读写和
向量检索记忆。魔法路径 `/search/<q>` 做向量 KNN、`/index/<key>` 写入即向量化入库，其余路径
照常落到真实文件系统层。

## 变更

### 工作区与工具链
- **`Cargo.toml`**：根 workspace，`exclude = ["link_repos"]`（关键修复——否则 link_repos 内
  libsql 的 `.workspace=true` 会被错误地解析到本仓库根，污染依赖图）。
- **`rust-toolchain.toml`**：钉住 Rust 1.85.0。
- **`crates/anyhome/Cargo.toml`**：lib + bin；`home = "=0.5.9"` MSRV pin（libsql-ffi 的 bindgen
  → which → home v0.5.12 需要 1.88）。
- **`Cargo.lock`**：随源码提交，锁定完整依赖图。
- **`scripts/bootstrap.sh`**：可复现地克隆并 pin `link_repos/{nfsserve,libsql}` 到指定 commit。
- **`scripts/e2e.sh`**：构建→启动→挂载→index/search 断言→清理的端到端冒烟（无 mount.nfs 时自动跳过）。

### DB 层（libsql 向量存储）
- **`crates/anyhome/src/db/mod.rs`**：`MemoryDb` —— schema 为
  `memories(id, key, text, emb FLOAT32(dim))` + `libsql_vector_idx(emb)`；`upsert`/`delete`/
  `search`。关键点：`vector_top_k(...)` 是表值函数，只暴露 `id/rowid` 而非基表列，因此
  `search` 用 `JOIN memories m ON m.rowid = v.id` 回查 `key,text`（file 行 99-101）。
- **`crates/anyhome/src/db/embedder.rs`**：`Embedder` trait + `DummyEmbedder`（确定性、无依赖；
  FNV+splitmix，trim 尾部空白以容忍 `echo` 的换行）。TODO-04（真实语义 embedder）待定。

### 文件系统层
- **`crates/anyhome/src/backend/mod.rs`**：模块导出 `RealFs` + `ROOT_ID`。
- **`crates/anyhome/src/backend/realfs.rs`**：`RealFs` 实现 `NFSFileSystem`，tokio::fs 包装真实
  目录；`fileid` = inode（根钉为 `ROOT_ID=1`，NFS 禁 0）；进程内 `id↔path` map；15 个 trait 方法。

### dispatcher（魔法路径分流）
- **`crates/anyhome/src/magic.rs`**：抽出 magic fileid 编码（bit63=MAGIC）、目录常量
  `SEARCH_DIR/INDEX_DIR/WHERE_DIR`、`MagicRegistry`、`percent_decode`、`magic_attr`、`slice_read`。
- **`crates/anyhome/src/dispatcher.rs`**：`AnyHomeFs{real, db, embedder, magic}` 实现 `NFSFileSystem`；
  根 lookup 识别 `search/index/where`；魔法文件 read→DB 查询、write→DB upsert；其余委托 `RealFs`。

### CLI 与库
- **`crates/anyhome/src/cli.rs`**：`Cli`（clap），`--fs-root/--db-path/--addr/--export/--dim`，
  抽到 lib 以便单测。
- **`crates/anyhome/src/main.rs`**：组装三层，`NFSTcpListener::bind+handle_forever` 单端口服务。
- **`crates/anyhome/src/lib.rs`**：模块声明与 re-export。

### 文档
- **`README.md`**：新增「使用」章节（构建/启动/挂载/读写/e2e），更新「状态」为已打通核心链路。
- **`plans/init-pro/`**：TODO-01..07 + index 设计文档。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| 向量 KNN 余弦排序（硬 gate） | `smoke_vector_top_k` | tests/db_vector.rs |
| DummyEmbedder 确定性 + 尾空白容忍 | `dummy_embedder_is_deterministic_and_matches` | tests/db_vector.rs |
| index→search 精确匹配 top-1 | `index_then_search_exact_match_top1` | tests/db_vector.rs |
| 删除记忆 | `delete_removes_a_memory` | tests/db_vector.rs |
| vec_to_json 格式 | `vec_to_json_is_valid_for_libsql` | tests/db_vector.rs |
| 根目录可见 magic 目录 | `magic_dirs_are_visible_at_root` | tests/anyhome_magic.rs |
| /index 写入 + /search 读取闭环 | `index_then_search_roundtrips_via_magic_paths` | tests/anyhome_magic.rs |
| magic 文件 attr 与可读 | `magic_file_has_reg_attr_and_is_readable` | tests/anyhome_magic.rs |
| 真实路径与 magic 并存 | `real_paths_still_work_alongside_magic` | tests/anyhome_magic.rs |
| /search 写入无副作用 | `writing_to_search_is_a_noop_but_does_not_error` | tests/anyhome_magic.rs |
| RealFs create/write/read | `create_write_read_roundtrip` | tests/realfs.rs |
| lookup 命中/未命中 | `lookup_finds_created_file` | tests/realfs.rs |
| mkdir/readdir/remove | `mkdir_readdir_remove` | tests/realfs.rs |
| rename | `rename_moves_a_file` | tests/realfs.rs |
| 根 fileid = ROOT_ID | `root_is_real_and_rootdir_is_root_id` | tests/realfs.rs |
| RealFs Clone 共享 map | `realfs_is_clone_and_shares_state` | tests/realfs.rs |
| CLI 全 flag 解析 | `cli_accepts_all_flags` | src/cli.rs |
| CLI 默认值 | `cli_defaults_are_sane` | src/cli.rs |
| port_of 解析 | `port_of_works` | src/cli.rs |

- 全量回归：`cargo test --workspace` → 19 passed; 0 failed; 0 ignored
- clippy：`cargo clippy --workspace --all-targets -- -D warnings` → 零警告
- 构建：`cargo build --workspace` → 干净
- 端到端：`scripts/e2e.sh` → 服务成功 bind 11111（沙箱无 mount.nfs，跳过真实挂载段）
- 行数（≤400 新文件 / ≤800 迭代）：
  dispatcher.rs 325、magic.rs 152、backend/realfs.rs 393、db/mod.rs 145、db/embedder.rs 106、
  cli.rs 71、main.rs 57、lib.rs 16、e2e.sh 69、bootstrap.sh 51

## Impact Surface
- 新增：可挂载的 NFSv3 记忆服务（首次实现，无既有调用方）。
- 调用方感知：`/search/<q>` 返回 `key\ttext\n` 每行一条；`/index/<key>` 写入即索引。
- 边界：仅本地单端口服务；不实现 NLM/文件锁；magic file handle 进程重启失效（设计如此）。

## Related Docs
- [plans/init-pro/index.md](../../../plans/init-pro/index.md)
- [plans/init-pro/TODO-01..07](../../../plans/init-pro/)
