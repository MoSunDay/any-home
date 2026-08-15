# anyhome 实现计划 · init-pro-8

- **project**: init-pro-8（写路径性能硬化 write-path performance hardening，语义保持）
- **created**: 2026-08-15
- **status**: done（TODO-46..50 全部完成）

## Goal

在**不动两条契约**（每次 WRITE 写穿 DB + fsync；写入即索引当次全文 embed）的前提下，
消除写路径的语义无关成本：量化基线 → 拆掉 760 行 `dispatcher.rs` 工程门 → 去除冗余
clone/回读 → 写缓冲升级 LRU + 字节上限。

## 契约红线（本迭代不触碰）

1. 每个 WRITE RPC 写穿 DB（upsert + commit fsync），无 close-time flush。
2. 每个 WRITE RPC 对**当次全文**做一次 embed（不做增量 embed / 延迟 embed）。
渐近写放大 O(N²/C) 受契约保护，属预期内；本迭代只清常数项。

## 关键技术决策

| 决策 | 结论 | 证据 |
|---|---|---|
| D1 dispatcher 拆分模式 | `dispatcher.rs`（trait impl + 结构）+ 子模块 `dispatcher/{dbops,routing,readdir}.rs` 各自持有 inherent `impl AnyHomeFs` 块（E0119 只限 trait impl 跨文件；inherent impl 可拆）。readdir trait 方法留薄委托 `magic_readdir -> Result<Option<..>>` | TODO-47 |
| D2 契约红线 | 不做延迟 embed / close-hook / fsync 松动；fattr 尺寸改由**本 WRITE 写入的字节数**直接得出（NFS WRITE 语义：返回 post-write 属性） | TODO-46/48 |
| D3 去整缓冲 clone 前提被推翻 | embed 是 async（`.await`）且 NFSFileSystem future 需 Send；std `MutexGuard` 非 Send → embed 输入必须离开锁存活 = 至少一次 O(N) 物化。接受单次 clone 为下界，仅消除**第二次 DB 回读**（spike 收缩条款） | TODO-48 |
| D4 缓冲淘汰 | 条目上限 256 保持 + 新增总字节上限 8 MiB；淘汰从「HashMap 迭代序任意」改为「最久未写优先」（std-only 逻辑时钟，锁内更新，≤256 条目 O(n) 扫描）；单缓冲自身超限时丢弃（DB 权威，下次 re-seed） | TODO-49 |
| D5 spike 基线 | 32KB×8 chunk：embed 恰 8 次（每 WRITE 一次）、总 embed 字节 = C·m(m+1)/2（受保护形状）；热路径每 WRITE 额外 1 次 get_text 回读（fattr），首写 +1 次 seed 探测 → m+1 次；TODO-48 后降为 1（仅首写探测） | TODO-46 + TODO-48 |

## 状态总表

| ID | 标题 | 状态 | 依赖 |
|----|------|------|------|
| TODO-46 | spike：写放大基线量化（tests/write_amplification.rs + get_text 计数器） | done | — |
| TODO-47 | dispatcher.rs 拆分（工程门，行为中性，单独 commit） | done | — |
| TODO-48 | 语义保持削减：消除 fattr 二次回读（clone 下界见 D3） | done | TODO-46, TODO-47 |
| TODO-49 | 写缓冲 LRU + 8 MiB 字节上限 | done | — |
| TODO-50 | 文档三件套 + 记忆同步 | done | TODO-46..49 |

## 全局验收 gate

- TODO-46..50 全 done。
- `cargo build --workspace` / `cargo clippy --workspace --all-targets -- -D warnings` /
  `cargo test --workspace` 双 feature 全绿。
- `cargo test` = **109 passed**（102 基线 + 2 write_amplification + 1 write_fattr + 4 magic
  LRU 单测）；`--features embed-fastembed` = **113 passed**。既有用例（durability /
  magic_write_semantics 等）零回归、断言原样通过。
- 行数：`dispatcher.rs` 455 / `dbops.rs` 135 / `routing.rs` 95 / `readdir.rs` 152（新文件均
  <400 门）；`magic.rs` 493 / `db/mod.rs` 658（<800 迭代门）。
- TODO-47 拆分独立 commit，可二分定位。
