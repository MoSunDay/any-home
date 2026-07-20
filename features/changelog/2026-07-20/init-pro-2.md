Commit: (working-tree, pre-initial-commit)

# init-pro-2：持久化 + 真实语义 embedder + /where FTS5 + recall 评测（TODO-08..13）

## 背景

init-pro 把 anyhome 落地为可挂载的 NFSv3 记忆服务 MVP，但留下几处占位：记忆不持久（`":memory:"`
重启即失）、`/where` 为预留空壳、`/search` 由 `DummyEmbedder` 驱动（仅字面匹配，无语义召回）、
`--export` 未接到 nfsserve、无真实挂载 e2e、无量化召回评测。本次按 `plans/init-pro-2` 收口这 6 项，
让记忆服务具备生产可用的持久化、真实语义检索与可回归的评测基线。

## 变更

### 持久化 / 重启存活（TODO-08）
- **`crates/anyhome/src/db/mod.rs`**：新增 `meta` 表（key-value，存 `embedding_dim`）；
  `reconcile_dim()` 启动时比对 CLI dim 与已落盘 dim，**不匹配即硬错误**（防止向量索引损坏）；
  `ensure_fts_populated()` 启动时按需重建 FTS5 索引。Connection 直接持有（libsql 内部 Arc，自动
  Send+Sync+Clone），无显式事务。

### /where FTS5 全文过滤（TODO-09）
- **`crates/anyhome/src/db/mod.rs`**：FTS5 虚拟表 `memories_fts` + 3 触发器（ai/ad/au）自动同步
  `memories → memories_fts`；`where_search()` 解析 `key:<前缀>` 分支走 LIKE、其余走 FTS5 MATCH；
  新增 `get_text()` / `list_keys()` 辅助查询。删除后 FTS5 由触发器自动清理。
- **`crates/anyhome/src/dispatcher.rs`**：新增 `run_where()`，readdir 重建 `/index` 列表自
  `db.list_keys()`，`/where/<cond>` 读路由到 `where_search()`。

### 真实语义 embedder（TODO-10，feature-gated）
- **`crates/anyhome/Cargo.toml`**：新增 `embed-fastembed` feature（默认 off）+ optional
  `fastembed = { version = "4", optional = true }`。
- **`crates/anyhome/src/db/fastembed.rs`**：`FastEmbedder`（`FASTEMBED_DIM=384`，
  `Qdrant/all-MiniLM-L6-v2-onnx`，`spawn_blocking` 跑推理避免阻塞 async runtime）。
- **`crates/anyhome/src/cli.rs`**：`EmbedderKind` enum（Dummy/Fastembed，`Default=Dummy`）+
  `FromStr`（大小写不敏感，未知值含 "unknown embedder" Err）+ `--embedder` flag。
- **`crates/anyhome/src/main.rs`**：`build_embedder()` 按枚举 + feature 构造 embedder。
- **`crates/anyhome/src/lib.rs`**：re-export `FastEmbedder`。
- **`Cargo.lock`**：fastembed 传递依赖（image/idna_adapter/icu_*/built）要求 1.86+，在此钉回
  1.85 兼容版本（image@0.25.5、idna_adapter@1.1.0、idna@1.0.3、built@0.7.7）。

### --export 接线 + mount（TODO-11）
- **`crates/anyhome/src/main.rs`**：`listener.with_export_name(&cli.export)`（nfsserve
  tcp.rs:171，`&mut self`），把导出路径接到 MOUNT 协议。

### 真实挂载 e2e（TODO-12）
- **`scripts/e2e.sh`**：新增 `--require-mount` 模式（`mount.nfs` 缺失时 exit 1 而非 exit 0，可做硬
  gate）；`ANYHOME_EMBEDDER` env 透传 `--embedder`（fastembed 时按 `--features` 构建 + 设
  `HF_HOME`/`HF_HUB_OFFLINE`）；新增 `/where` 断言。
- **`scripts/fetch-model.sh`**：预取 ONNX 模型到标准 HF cache 布局，支持 `--mirror`（hf-mirror.com）
  与 `HF_ENDPOINT`，已缓存则跳过；解决 HuggingFace 主站不可达 / hf-hub 镜像下载失败。

### recall 评测脚手架（TODO-13）
- **`crates/anyhome/tests/recall.rs`**：10 对（query, key+text）改写集；`recall_at_k()` 计算 top-K
  命中率。两个测试：① dummy baseline recall@3 < 0.5（证明测试有区分力，非橡皮图章）
  ② fastembed recall@3 ≥ 0.7（真实语义召回有效）。

### CLI 测试恢复与补强
- **`crates/anyhome/src/cli.rs`**：恢复 init-pro 期间被删的 3 个 CLI 测试并适配新行为，新增 3 个
  覆盖 `--embedder` / `EmbedderKind::FromStr`（正常+错误路径）/ `port_of` 的结构化测试。

### 文档
- **`README.md`**：更新 `/where`（FTS5 + key 前缀）、embedder 说明（feature + `--embedder`）、
  新增「持久化」章节、状态章节改为已解决（35/39 用例）。
- **`plans/init-pro-2/`**：index + TODO-08..13 设计文档（init-pro 格式）。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| 重启后记忆存活 | `file_db_survives_reopen` | tests/db_persistence.rs |
| dim 持久化、相同 dim 重开 OK | `reopen_with_same_dim_is_fine` | tests/db_persistence.rs |
| dim 不匹配 = 硬错误 | `reopen_with_mismatched_dim_is_rejected` | tests/db_persistence.rs |
| /where 单词匹配 | `where_matches_a_single_term` | tests/where_fts.rs |
| /where 跨文档共享词匹配 | `where_matches_shared_word_across_docs` | tests/where_fts.rs |
| /where key 前缀过滤 | `where_key_prefix_filter_matches_keys` | tests/where_fts.rs |
| /where 删除后不再命中 | `where_no_longer_hits_after_delete` | tests/where_fts.rs |
| /where 空条件返回空 | `where_empty_condition_returns_nothing` | tests/where_fts.rs |
| dispatcher /where 全文 | `where_fulltext_returns_matching_keys_only` | tests/anyhome_magic.rs |
| dispatcher /where key 前缀 | `where_key_prefix_filters_by_key` | tests/anyhome_magic.rs |
| dispatcher 重启存活 | `index_listing_and_text_survive_restart` | tests/anyhome_magic.rs |
| embedder dim=384 | `dim_is_384` | tests/fastembed_semantic.rs |
| embedder 确定性 | `embeddings_are_deterministic` | tests/fastembed_semantic.rs |
| 语义不等式 cos(hi)>cos(banana) | `semantic_inequality_holds` | tests/fastembed_semantic.rs |
| dummy recall@3 < 0.5（区分力） | `dummy_embedder_baseline_fails_paraphrase_recall` | tests/recall.rs |
| fastembed recall@3 ≥ 0.7 | `fastembed::fastembed_clears_recall_bar` | tests/recall.rs |
| CLI 全 flag 解析（含 --embedder） | `cli_accepts_all_flags` | src/cli.rs |
| CLI --embedder fastembed 端到端 | `cli_accepts_embedder_fastembed_flag` | src/cli.rs |
| CLI 默认值 | `cli_defaults_are_sane` | src/cli.rs |
| EmbedderKind FromStr 正常路径 | `embedder_kind_fromstr_parses_known_backends` | src/cli.rs |
| EmbedderKind FromStr 错误路径 | `embedder_kind_fromstr_rejects_unknown` | src/cli.rs |
| port_of 行为 | `port_of_works` | src/cli.rs |

- 全量回归：`cargo test --workspace` → **35 passed; 0 failed**
- fastembed 回归：`cargo test --workspace --features embed-fastembed` → **39 passed; 0 failed**
  （recall@3 fastembed=1.00，dummy=0.30；语义不等式 cos(hello,hi)=0.807 > cos(hello,banana)=0.288）
- clippy：`cargo clippy --workspace --all-targets -- -D warnings` → 零警告（两 feature 配置均过）
- 行数：`db/mod.rs` 349 ≤ 400；`dispatcher.rs` 358 ≤ 400；其余文件均 ≤ 400

## Impact Surface

- **对用户可感知**：记忆现在持久化到 `--db-path` 文件，重启存活；`/where/<词>` 可用（FTS5 全文 +
  `key:<前缀>`）；`--embedder fastembed`（需 `--features embed-fastembed` 构建）开启真实语义检索；
  `--export` 决定 NFS 导出路径。
- **不影响**：NFS 协议边界、文件系统层分流逻辑（RealFs）、向量 KNN 检索路径（`/search` 仍可用，
  DummyEmbedder 仍为零依赖默认）。
- **回归面**：DB schema 变更（新增 meta/FTS5 表）由 db_persistence + where_fts 覆盖重启路径；
  CLI 解析层由 6 个 cli 单元测试覆盖。

## Related Docs

- [plans/init-pro-2/index.md](../../plans/init-pro-2/index.md)
- [既有相关 changelog：init-pro](../2026-07-19/init-nfs-memory-service.md)
