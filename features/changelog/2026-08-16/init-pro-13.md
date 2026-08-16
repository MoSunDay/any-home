# init-pro-13：优雅停机 + WAL checkpoint、ANN 索引退役、fastembed 离线 fail-fast、受控再部署（TODO-72..77）

## 背景

TODO-72 取证实锤两处部署隐患：① `anyhome.db` 主库仅 4 096 B 而 `-wal`
达 2 187 752 B——自部署以来已提交写全在 WAL，从未 checkpoint 回主库，
裸拷主库即漏数据；② init-pro-12 起 `search` 已是 agent 分区暴力 KNN，
ANN 索引 `memories_vec_idx` 零查询命中、只剩逐写维护成本。另有 fastembed
侧：hf-hub 0.4.3 对 hf-mirror.com 无超时，模型不在缓存时初始化会无限挂起。

## 变更

1. **优雅停机（TODO-73）**：`MemoryDb::checkpoint()`（新增，只增签名）执行
   `PRAGMA wal_checkpoint(TRUNCATE)`；`main` 以 `tokio::select!` 监听
   SIGINT/SIGTERM → 记日志 → checkpoint → 退出码 0。durability +3 例。
2. **schema v2（TODO-74）**：迁移末尾无条件 `DROP INDEX IF EXISTS
   memories_vec_idx`，级联清影子表与元数据行；退役索引丢掉的写时维度校验由
   新增 `check_emb_dim`（upsert 前置）补回，报错文案不变。`schema_v2.rs` +2 例。
3. **fastembed fail-fast（TODO-75）**：`FastEmbedder` 构造前做纯函数缓存探测
   （`HF_HOME`/`FASTEMBED_CACHE_DIR`/默认目录 → `refs/main` + 模型文件清单），
   缺失即带排查指引报错退出，运行时零联网。feature 门控 +8 例；
   `fetch-model.sh` 修 `refs/main` 尾换行；`e2e.sh` 加 fail-fast 守卫。
4. **受控再部署（TODO-76）**：旧二进制 Phase-A 数据安全回归 PASS → release
   重建 → 同参数换绑 → 部署库实弹迁移（version=2、向量残留清零、记忆字节
   全等）→ SIGTERM 演练（优雅日志、wal 2 665 672→0 B、主库 4 096→94 208 B、
   重启字节全等）→ Phase-B 矩阵三零残留 PASS → `scripts/e2e.sh
   --require-mount 21111` 全绿。
5. **文档与对账（TODO-77）**：README 计数 217/221→222/234、ANN 口径、
   fastembed 离线语义、新增「停止与备份」运维小节；`plans/init-pro-13/` 全套记录。

## 不变量（未动）

`MemoryDb`/`MagicRegistry` 公共签名只增不改；每个 WRITE RPC 写穿 DB；每次
数据变更单次 embed；`fs.db.*` 直连路径；表/列/FTS 结构；
`synchronous=FULL` + WAL 耐久基线；部署记忆字节不变（对 `tmp/audit67_dump.json`）。

## 验证

- `cargo test --workspace` = 222 passed；`--features embed-fastembed` =
  234 passed（`fastembed_semantic` 无挂起）；clippy `-D warnings` 双组合零警告。
- live：部署实例已切新二进制，e2e 全绿，SIGTERM 演练后 WAL 收敛、重启字节全等。
