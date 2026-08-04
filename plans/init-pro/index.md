# anyhome 实现计划 · init-pro

- **project**: init-pro（anyhome 首个实现周期）
- **created**: 2026-07-19
- **status**: done（TODO-01..07 全部完成）

## 关键技术决策（3 轮只读调研验证，附 file:line 证据）

| 决策 | 结论 | 关键证据 |
|---|---|---|
| 工作区形态 | 根 workspace member=`crates/anyhome`；link_repos 为 path 依赖、**绝不**列 members | libsql/libsql 内层无 `[workspace]`、是父 ws 合法 member；nfsserve 无 `[workspace]` |
| libSQL core-only | `default-features=false, features=["core"]`，避开 tonic/hyper/rustls/tower/aws-sdk-s3 | `libsql/Cargo.toml:60-67` core 仅拉 libsql-sys+bitflags+bytes+futures+parking_lot |
| **vector 可用性（最大风险，已排除）** | core-only 下 `vector()`/`vector_top_k()`/`FLOAT32(N)`/`libsql_vector_idx()` 确定可用 | `build.rs:199-218` 不传 `-DSQLITE_OMIT_VECTOR`；`vector*.c` 全 `#ifndef` 守卫；amalgamation 含 vector C（96 处命中） |
| Rust 版本 | 钉 `1.85.0`（与 `link_repos/libsql/rust-toolchain.toml` 对齐 + 可复现） | 注：libsql edition=2021；resolver=3 需 cargo>=1.85；实际 >=1.85 均可，钉 1.85 为复现 |
| `.cargo/config.toml` | **不需要** `--cfg tokio_unstable`（core-only 不依赖；全 libsql `.rs` 源码 0 命中该 cfg） | `grep tokio_unstable` 仅命中 `.cargo/config.toml` 自身 |
| link_repos 钉 commit | bootstrap checkout 上游 base 再叠 memory commit | libsql base `6f451a1f`->`faf79716`；nfsserve base `c00c3614`->`abe65c59` |
| NFS 集成点 | impl `NFSFileSystem`（15 required）；`lookup/getattr/read/readdir/write/create` 为分流点 | `nfsserve/src/vfs.rs:94-261` |
| fileid 划分 | 真实 FS 用低位 inode（根=1，规避 `vfs.rs:93` 的 0 保留）；魔法用 `0x8000_0000_0000_0000\|key` | 默认 `id_to_fh/fh_to_id`（`vfs.rs:222-242`）对 id 零假设；高位段 round-trip 安全 |
| Connection 并发 | `MemoryDb{conn: Connection}`，**不包 Mutex**；共享用 `conn.clone()`；**不用显式 transaction**（单条 execute/query 自带隐式事务） | `connection.rs:131-135` 内部 `Arc<dyn Conn+Send+Sync>`，自动 Send+Sync+Clone |
| 向量检索 SQL | `FLOAT32(N)` N 字面量拼接；索引 `libsql_vector_idx(col)` 无 USING、默认 metric=cosine；插 `vector(?)` 包 JSON 串；查 `vector_top_k('索引名', vector(?), k)` 第一参=索引名、结果无 distance 列 | `integration_tests.rs:783,792,815,843`；`libsql_vector_index.test:113,117` |
| 启动入口 | `NFSTcpListener::bind(ipstr, fs).await`（值传递，泛型单态化，内部自包 Arc）-> `handle_forever()` | `tcp.rs:107,196-214` |
| **mount 命令（e2e 硬约束）** | nfsserve **无真 rpcbind**（`portmap_handlers.rs:73-88` 假实现 multiplex 在业务端口，不监听 111）；必须 `-o port=,mountport=,nolock` | `rpcwire.rs:48-53`；README:131-137,37,43 |

## 状态总表 + 依赖图

| ID | 标题 | 状态 | 依赖 |
|----|------|------|------|
| TODO-01 | 工作区骨架 + 依赖编译验证 | done | - |
| TODO-02 | 真实 FS 直通后端 | done | TODO-01 |
| TODO-03 | DB schema + MemoryDb + Embedder 抽象 | done | TODO-01 |
| TODO-04 | 真实 Embedder（语义模型接入） | done | TODO-03 |
| TODO-05 | 魔法路径路由（分流 + DB 查询路径） | done | TODO-02, TODO-03 |
| TODO-06 | 二进制 + CLI 配置 | done | TODO-05 |
| TODO-07 | e2e 脚本 + 使用文档 | done | TODO-06 |

```
TODO-01 ─┬─> TODO-02 ──┐
         │             ├──> TODO-05 ──> TODO-06 ──> TODO-07
         └─> TODO-03 ──┤
               └─> TODO-04  (增强，非阻塞：DummyEmbedder 让 TODO-05 独立推进)
```

## 全局验收 gate
- TODO-01..07 全 done（真实 Embedder 已在 init-pro-2 TODO-08..13 交付）
- `cargo build` / `cargo clippy --workspace --all-targets -- -D warnings` / `cargo test --workspace` 全绿
- **TODO-03 vector 冒烟测试必须非-ignore 且通过**（应对零 CI 保护：core libsql 的 vector 测试全 `#[ignore]`）
- e2e 脚本（TODO-07）Linux 手动跑通，mount 命令带 `port=/mountport=/nolock`

## 已锁定的开放决策
- **R4（魔法文件 readdir 可见性）= 可见**：根 `readdir` 拼入 `/search /index /where` 三个魔法目录条目。理由：README 定位「agent 用 ls 探索记忆」，隐藏接口违背产品语义。
- **TODO-04 = done**：真实语义 Embedder 已在 init-pro-2（TODO-08..13）交付；init-pro 主线 DummyEmbedder 让 TODO-05 独立推进的历史决策已闭环。

## 环境就绪度（3 轮实测）
- cc/gcc 11.4.0 ✓ | cmake 3.22.1（冗余）| make ✓ | pkg-config ✓ | rustup ✓ | rustc 1.85.0 已装 ✓ | network(crates.io) ✓ | cached 446 crates
- mount.nfs / nfs-common / rpcbind **全缺**（仅影响 TODO-07 e2e，不影响 TODO-01..06）
- libclang 缺失但 default 构建路径不触达（仅 `LIBSQL_DEV` 时才需 bindgen）
