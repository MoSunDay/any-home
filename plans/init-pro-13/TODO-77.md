# TODO-77 · 文档三件套 + 计数对账

## 产出

- `plans/init-pro-13/`：`index.md` + TODO-72..77 六份记录（本文件在内）。
- `features/changelog/2026-08-16/init-pro-13.md`。
- `README.md`：
  - 计数对账：217→**222**（默认）、221→**234**（embed-fastembed），
    「运行测试」与「状态」两处同步；
  - 架构图与能力清单的 ANN 口径更新（`FLOAT32(dim)` 存储 + agent 分区暴力
    KNN，schema v2 起索引退役）；
  - 嵌入后端表：fastembed 运行时不联网、缓存缺失启动即 fail-fast；
  - 新增「2.1 停止与备份」小节：SIGTERM 优雅收敛 + 停机单文件备份 /
    不停机 `sqlite3 .backup`（警示勿裸 cp 未收敛 db）；
  - 能力清单补「优雅停机」条目。

## 计数对账（相对 init-pro-12 收尾基线 217/221）

| 变化 | 归属 | 数量 |
|------|------|------|
| `durability.rs` checkpoint 三例 | TODO-73 | +3 |
| `tests/schema_v2.rs` 新文件两例 | TODO-74 | +2 |
| `db/fastembed.rs` 缓存探测/真值表（feature 门控） | TODO-75 | +8 |

默认：217 + 3 + 2 = **222**；fastembed：221 + 3 + 2 + 8 = **234**
（`cargo test --workspace` 双组合实测一致，0 failed）。

## 验收口径

- 双组合测试数与上表一致；clippy `-D warnings` 双组合零警告。
- 部署实例已切新二进制且 live e2e 全绿（见 TODO-76.md）。
