# init-pro-13 · 优雅停机 + WAL checkpoint、ANN 索引退役、fastembed 离线 fail-fast、受控再部署（TODO-72..77 全部完成）

## Goal

在**不动任何对外语义契约**（`MemoryDb`/`MagicRegistry` 公共签名只增不改、每个
WRITE RPC 写穿 DB、每次数据变更单次 embed、`fs.db.*` 直连路径不变、
表/列/FTS 结构不变、`synchronous=FULL`+WAL 耐久基线不变）的前提下：
① 取证确认部署 WAL 未收敛与 ANN 索引写放大；② 优雅停机 + WAL checkpoint；
③ schema v1→v2 退役闲置 ANN 索引；④ fastembed 离线 fail-fast（启动即报错，
不挂起）；⑤ 受控再部署 + live e2e + 文档/计数对账。

## 摸底结论（TODO-72 取证先行）

1. **部署 WAL 未收敛**：`anyhome.db` 主库仅 4096 B，`anyhome.db-wal` 却达
   2 187 752 B —— 自部署以来所有已提交写全在 WAL 里，从未 checkpoint 回主库。
   进程被 SIGKILL 不丢数据（`synchronous=FULL`），但裸 `cp` 主库会漏数据。
2. **ANN 索引闲置**：init-pro-12（TODO-68）起 `search` 已是 agent 分区暴力
   KNN，`memories_vec_idx` 零查询命中，只剩逐写维护成本。
3. **级联删除事实**（实测钉死）：`DROP INDEX memories_vec_idx` 会级联删
   `memories_vec_idx_shadow` + `memories_vec_idx_shadow_idx` +
   `libsql_vector_meta_shadow` 中对应行；全局注册表 `libsql_vector_meta_shadow`
   本身保留为空表（惰性基础设施，零写成本）。
4. **fastembed 挂起根因**：hf-hub 0.4.3 对 `hf-mirror.com` 无超时；
   huggingface.co reset 很快但 mirror 会无限挂起 → 必须离线 fail-fast。
5. 产物：`tmp/audit72/{sqlite_master.txt,vec_idx_footprint.txt,deploy_cmdline.txt,
   anyhome.db.bak,anyhome.db-wal.bak}`。

## 变更

1. **TODO-73 优雅停机**：`MemoryDb::checkpoint()`（`PRAGMA
   wal_checkpoint(TRUNCATE)`，新增只增签名）；`main.rs` 用 `tokio::select!`
   监听 SIGINT/SIGTERM → 记日志 → checkpoint → 退出码 0。durability 测试 +3。
2. **TODO-74 schema v2**：`SCHEMA_VERSION=2`，迁移末尾无条件
   `DROP INDEX IF EXISTS memories_vec_idx`；**计划外但必须**：退役索引同时
   丢掉了写时维度校验，新增 `check_emb_dim`（schema.rs）并在 `upsert`
   （crud.rs）调用，保住 "vector dimensions are different" 报错语义。
   `tests/schema_v2.rs` 新文件 +2。
3. **TODO-75 fastembed fail-fast**：`FastEmbedder::new`/`with_model` 先做
   纯函数缓存探测（`HF_HOME`/`FASTEMBED_CACHE_DIR` → 默认
   `~/.fastembed_cache`，`refs/main` + 模型文件清单），缺失即启动报错退出，
   运行时零联网。feature 门控测试 +8；`scripts/fetch-model.sh` 修
   `refs/main` 尾换行（`printf '%s'`）；`scripts/e2e.sh` 加 fail-fast 守卫
   与 `HF_HOME` `/hub` 子目录修正。
4. **TODO-76 受控再部署**：旧二进制 Phase-A 数据安全回归（`regress-13-*`，
   PASS）→ release 重建 → 受控换绑 → 新二进制上迁移实弹验证（部署库
   version=2、向量残留清零、记忆字节全等）→ SIGTERM 演练（优雅日志 +
   wal 2 665 672→0 B、主库 4 096→94 208 B + 重启后字节全等）→ Phase-B 全量
   矩阵 + triple zero-residue（PASS）→ `scripts/e2e.sh --require-mount 21111`
   全绿。
5. **TODO-77 文档三件套 + 计数对账**：本目录记录、changelog、README
   （计数 217/221→222/234、架构/状态小节的 ANN 退役口径、fastembed 离线
   语义、新增「停止与备份」运维小节）。

## 验收口径

- `cargo test --workspace` = **222** passed / 0 failed。
- `cargo test --workspace --features embed-fastembed` = **234** passed / 0 failed
  （`fastembed_semantic` 3 例无挂起——fail-fast 守卫生效）。
- `cargo clippy --all-targets -- -D warnings` 两种 feature 组合均零警告。
- live：部署实例已切新二进制（同参数），`/search` `/where` `/index` 与
  agent 鉴权路径 e2e 全绿；停机演练后部署记忆与 `tmp/audit67_dump.json`
  字节全等。

## TODO 索引

- [TODO-72](TODO-72.md) 部署取证：WAL 未收敛 + ANN 索引足迹（已完成）
- [TODO-73](TODO-73.md) 优雅停机 + WAL checkpoint（已完成）
- [TODO-74](TODO-74.md) schema v2：ANN 索引退役 + 维度门禁保留（已完成）
- [TODO-75](TODO-75.md) fastembed 离线 fail-fast（已完成）
- [TODO-76](TODO-76.md) 受控再部署 + live e2e（已完成）
- [TODO-77](TODO-77.md) 文档三件套 + 计数对账（本文件在内）
