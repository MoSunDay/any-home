# TODO-75 · fastembed 离线 fail-fast

## 背景

fastembed 首次初始化会经 hf-hub 0.4.3 拉模型：huggingface.co 被 reset 时
报错尚快，但 hf-mirror.com **无超时机制、无限挂起**。部署环境网络不可控，
必须改为「缓存不在即启动报错」，运行时零联网。

## 产出

- `src/db/fastembed.rs` 67→277（+210，feature `embed-fastembed` 门控内）：
  - 纯函数三件套：`hf_cache_root`（`HF_HOME` → `FASTEMBED_CACHE_DIR` →
    `~/.fastembed_cache`）、`hf_model_dir_name`（repo 名转 `models--org--name`
    布局）、`cached_snapshot_complete`（`refs/main` 指向的 snapshot 目录内
    模型文件清单齐全）。
  - `classify_model_cache`：`(cache_complete, offline)` → 显式状态枚举，
    真值表由单测钉死。
  - `ensure_model_cache_or_fail_fast`：在 `FastEmbedder::new`/`with_model`
    最前面调用——缓存缺失即返回带排查指引（`scripts/fetch-model.sh`）的
    错误，进程启动即退出，**绝不进入下载路径**。
  - 单测 +8：cache root 三级回退（3）、snapshot 探测收/拒（4，含
    `refs/main` 尾换行拒绝）、classify 真值表（1）。
- `scripts/fetch-model.sh`：写 `refs/main` 改 `printf '%s'`（去尾换行，
  与 hf-hub 解析对齐）。
- `scripts/e2e.sh` 445→460（+15）：fastembed 路径加 fail-fast 守卫——先断言
  缓存就位再启服务；修正 `HF_HOME` 需指向含 `/hub` 的目录。

## 实测

- `cargo test --features embed-fastembed`：`fastembed_semantic` 3 例全绿、
  无挂起；`--features embed-fastembed` 总盘 234 passed。
- 缓存缺失场景由 classify/probe 单测钉死（不触网）。
