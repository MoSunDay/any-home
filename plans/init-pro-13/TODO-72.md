# TODO-72 · 摸底取证（只读先行）

全部只读完成，未改任何源文件。产物落 `tmp/audit72/`（不入库）。

## ① 部署库 WAL / 主库字节 + 索引页占用基线

活体进程 pid **701435**，flags（逐字）：
`./target/release/anyhome --fs-root ./anyhome-fs --db-path ./anyhome.db --addr 127.0.0.1:11111`

| 文件 | 字节 |
|------|------|
| `anyhome.db`（主库） | **4096**（仅 1 页，空壳） |
| `anyhome.db-wal` | **2187752**（≈2.1 MB） |
| `anyhome.db-shm` | 32768 |

只读快照（`mode=ro` + 单事务 pin WAL）：`journal_mode=wal`、`page_size=4096`、
逻辑 `page_count=23`、`freelist_count=6`、`memories` 2 行、`meta`: `dim=64`、
`schema_version=1`。全部已提交数据躺在 WAL 里，主库从未 checkpoint——坐实
"cp anyhome.db 式备份会静默丢数据"。dump：`tmp/audit72/sqlite_master.txt`、
`vec_idx_footprint.txt`、备份 `anyhome.db.bak`/`anyhome.db-wal.bak`。

`dbstat` 页占用（ANN 索引相关）：
`memories_vec_idx_shadow`=20480、`memories_vec_idx`=4096、
`memories_vec_idx_shadow_idx`=4096、`libsql_vector_meta_shadow`=4096。
vec shadow 仅 2 行（对应 2 条 memories）。即闲置 ANN 索引及其 shadow 对象
合计占用约 32 KB 页空间，纯写维护成本、零查询消费。

## ② nfsserve 停机面

`NFSTcpListener::handle_forever`（`link_repos/nfsserve/src/tcp.rs:196`）是
`loop { accept().await? ; tokio::spawn(process_socket) }`，**无 stop API、无信号
处理**（只读依赖，不改上游）。→ 停机方案定为 `tokio::select!` 竞争取消。
边界：取消瞬间已 spawn 的 in-flight 连接任务会继续跑到进程退出；已提交写靠
`synchronous=FULL` 保证持久，未提交即未写。

## ③ tokio signal

根 `Cargo.toml:24` `tokio = { version = "1", features = ["full"] }`，`full`
已含 `signal`（unix SIGINT/SIGTERM 可用）。记录在案，无需加依赖。

## ④ hf-hub/fastembed 无缓存行为 → TODO-75 走"修复（fail-fast）"

- 本机网络实测：`huggingface.co` SSL 0.07s 被重置（connect reset）；
  `hf-mirror.com` 丢包**挂死**（timeout 124）——复现 init-pro-12 的"挂死"。
- 无缓存时 `FastEmbedder::new()` 直接 `Err`（约 1s，因为默认 endpoint 快速
  reset），`fastembed_semantic` 3 例自跳过；但换 mirror/dropping 网络就挂死。
- 依赖链：fastembed 4.9.1 → hf-hub 0.4.3。hf-hub 0.4.3 **无 offline 开关、
  无超时配置**；`ApiRepo::get()` 缓存未命中即走 ureq 下载，行为不受我们控制。
- 结论：挂死点在依赖内部，无法在库内修。→ TODO-75 采 **fail-fast 纯函数缓存
  探测**：`FastEmbedder::new()` 前先探测 HF 缓存是否就绪，未就绪即返回可操作
  错误（指路 `scripts/fetch-model.sh` / `HF_HOME`），把"挂死"变"快速可诊断失败"。
  这是可行的修复路径，非降级。

## 决策落点

- 73：`tokio::select!` 竞争 `handle_forever` 与 SIGINT/SIGTERM。
- 74：`DROP INDEX IF EXISTS memories_vec_idx` 单语句即可连带清理 shadow 表 /
  shadow_idx / `libsql_vector_meta_shadow` 元数据行（已在 libsql-sqlite3 C 源
  `build.c`→`vectorIndexDrop`→`diskAnnDropIndex`+`removeIndexParameters` 验证）。
- 75：纯函数缓存探测 + fail-fast，配套单元测试。
