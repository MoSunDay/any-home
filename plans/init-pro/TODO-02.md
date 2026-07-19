# TODO-02 真实 FS 直通后端（文件系统层 MVP）

## 目标
实现 `AnyHomeFs` 的真实 FS 一侧：指向磁盘真实目录，挂载后可 `ls/cat/echo>/mkdir`
管理「可穷举记忆文件」。魔法分流留给 TODO-05。

## 核心实现
- `backend/realfs.rs`：`RealFs{root: PathBuf}` 封 `tokio::fs`。**直接借鉴 `mirrorfs.rs`**
  （已是近完整真实 FS impl，`examples/mirrorfs.rs:298-605` 全 15 方法有 tokio::fs 调用）。
- `dispatcher.rs`：`AnyHomeFs{real: RealFs}`（本 TODO 只持 real，TODO-05 加 db/embedder）。
- fileid 用 `MetadataExt::ino()`；**根目录 fileid=1**（规避 `vfs.rs:93` 的「0 保留」；mirrorfs 用 0 属已知违规）。
- 15 required 方法落地：capabilities/root_dir/lookup/getattr/setattr/read/write/
  create/create_exclusive/mkdir/remove/rename/readdir/symlink/readlink。
- `lookup` 处理 `.`/`..`（返 dirid 自身 / parent，参考 `demo.rs:169-175`）。
- `readdir` **不返** `.`/`..`（NFS3 不要求，客户端自合成）。
- setattr 用 `fs_util::path_setattr`（`fs_util.rs:122-164`）。
- **不重写** id_to_fh/fh_to_id/path_to_id/fsinfo/readdir_simple/serverid（default 复用）。
- `examples/mount_realfs.rs`：`NFSTcpListener::bind+handle_forever`。

## 验收手段
集成测试（`tempdir` 直调 trait 方法）：
- create->write->read round-trip
- mkdir + readdir（条目正确，无 `.`/`..`）
- remove / rename 反映到后续 lookup/readdir
- `cargo test -p anyhome` 绿

## 状态
pending

## 证据
（测试名 + passed 数；e2e 操作）

## 卡点
- `fattr3` 字段多易错——**直接用** `fs_util::metadata_to_fattr3`（`fs_util.rs:39`），不自拼。
- **已知限制（写 changelog）**：`path_setattr` 的 uid/gid 是 no-op（`fs_util.rs:146-151` 只 debug 打印），
  生产 chown 暂不补。
- inode 在 tmpfs 可能不稳定——generation 机制本就要求重启失效，可接受。
- `lookup` 返回**仅 `fileid3`**（非 `FileidAndName+fattr3`；attr 由客户端后续 getattr 或 READDIRPLUS handler 补）。

## 依赖
TODO-01
