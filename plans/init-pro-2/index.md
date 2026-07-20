# anyhome 实现计划 · init-pro-2

- **project**: init-pro-2（anyhome 持久化 + 真实语义 + e2e 收尾周期）
- **created**: 2026-07-20
- **status**: done（全部 TODO-08..13 完成）

## 关键技术决策

| 决策 | 结论 | 证据 |
|---|---|---|
| 持久化策略 | DB 文件持久化 + `meta` 表存 dim + 启动时 FTS5 重建；dim 不匹配=硬错误 | `db/mod.rs reconcile_dim()` / `ensure_fts_populated()` |
| `/where` 实现 | SQLite FTS5 虚拟表 + 触发器同步 + `key:<前缀>` 分支 | `db/mod.rs memories_fts` + triggers ai/ad/au |
| 真实 embedder | fastembed（本地 ONNX，`Qdrant/all-MiniLM-L6-v2-onnx`，dim=384），feature-gated（`embed-fastembed`），默认 DummyEmbedder 零依赖 | `db/fastembed.rs`，`Cargo.toml` feature |
| 模型离线分发 | `scripts/fetch-model.sh` 预取到标准 HF cache 布局 + `HF_HUB_OFFLINE=1` | `scripts/fetch-model.sh`，`tests/fastembed_semantic.rs` auto-detect |
| MSRV 对齐 | fastembed 传递依赖（image/idna_adapter/built）要求 1.86+，在 Cargo.lock 钉回 1.85 兼容版本 | `Cargo.lock`: image@0.25.5, idna_adapter@1.1.0, idna@1.0.3, built@0.7.7 |
| e2e mount 约束 | `mount.nfs` 缺失时 `scripts/e2e.sh` 自动跳过（exit 0），`--require-mount` 模式可做硬 gate | `scripts/e2e.sh` |

## 状态总表 + 依赖图

| ID | 标题 | 状态 | 依赖 |
|----|------|------|------|
| TODO-08 | 持久化 / 重启存活 | done | TODO-03(init-pro) |
| TODO-09 | `/where` FTS5 全文过滤 | done | TODO-08 |
| TODO-10 | 真实语义 embedder（fastembed/ONNX） | done | TODO-08 |
| TODO-11 | `--export` 接线 + mount | done | TODO-06(init-pro) |
| TODO-12 | 真实挂载 e2e | done（沙箱无 mount.nfs，自动跳过） | TODO-11 |
| TODO-13 | recall 评测脚手架 | done | TODO-10 |

```
TODO-08 ─┬─> TODO-09
         ├─> TODO-10 ──> TODO-13
         └─> TODO-11 ──> TODO-12
```

## 全局验收 gate
- TODO-08..13 全 done。
- `cargo build --workspace` ✓ + `cargo build --workspace --features embed-fastembed` ✓
- `cargo clippy --workspace --all-targets -- -D warnings` ✓（两种 feature 配置均绿）
- `cargo test --workspace` = 35 passed（无 features；含 6 个 cli 单元测试）
- `cargo test --workspace --features embed-fastembed`  = 39 passed（3 fastembed_semantic + 2 recall gated 测试新增；含 6 个 cli 单元测试）
- TODO-10 语义不等式：cos(hello,hi)=0.807 > cos(hello,greetings)=0.700 > cos(hello,banana)=0.288
- TODO-13 recall@3：fastembed=1.00（过 0.7 bar），dummy=0.30（不过 0.5 bar，预期失败——证明测试有区分力）

## 已锁定的开放决策
- **init-pro TODO-04（真实 Embedder）= 已解锁**：TODO-10 用 fastembed 本地 ONNX 落地，feature-gated、默认零依赖，语义不等式 + recall 双测试证明有效。
- **R4（魔法文件 readdir 可见性）= 可见**：沿用 init-pro 决策，`/search /index /where` 三魔法目录在根 `readdir` 可见。
