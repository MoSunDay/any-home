# anyhome 实现计划 · init-pro-3

- **project**: init-pro-3（魔法路径正确性与健壮性 magic-path robustness）
- **created**: 2026-07-26
- **status**: done（TODO-00 + TODO-14..20 全部完成）

## Goal

让 `/search`、`/index`、`/where` 三条魔法路径的**写入与句柄语义达到生产可用正确性**：
消除写入损坏、静默成功、虚报属性、重启丢失句柄等问题。**不引入新功能**，专注把已交付能力做对、做稳。

## 关键技术决策

| 决策 | 结论 | 证据 |
|---|---|---|
| 写入缓冲策略 | 进程内 per-fileid 缓冲，按 offset splice 后**每次 WRITE 即 upsert 全文**（牺牲写入放大换正确性，NFS 无可靠 close 回调） | `magic.rs write_into` + `dispatcher.rs write` |
| `/index` 句柄重启恢复 | fileid 是确定性 `FNV-1a(name)`，内存缺失时扫描 DB keys 反查并缓存 | `dispatcher.rs index_name_for` |
| `/search`、`/where` 大小 | 内容随查询动态生成，getattr 维持**虚拟上限**（`MAGIC_FILE_SIZE`），避免每次 getattr 执行检索；`/index` 报真实字节 | `dispatcher.rs getattr` / `magic_attr_sized` |
| 只读端点错误码 | `/search`、`/where` 写/setattr 返回 `NFS3ERR_ROFS` | `dispatcher.rs write/setattr` |
| 二进制安全 | DB `text` 列需 UTF-8 → 非 UTF-8 写入显式返回 `NFS3ERR_INVAL`（不做 lossy 替换） | `dispatcher.rs write` |
| bootstrap 可复现 | 原"memory baseline"覆盖提交未发布上游 → pin 上游 HEAD（= base）+ `--depth 1` | `scripts/bootstrap.sh` |

## 状态总表 + 依赖图

| ID | 标题 | 状态 | 依赖 |
|----|------|------|------|
| TODO-00 | 环境就绪（重建 link_repos，基线绿） | done | — |
| TODO-14 | write 支持 offset | done | TODO-00 |
| TODO-15 | create_exclusive 独占语义 | done | TODO-00 |
| TODO-16 | 只读端点返回 ROFS | done | TODO-00 |
| TODO-17 | getattr 真实大小 | done | TODO-00 |
| TODO-18 | 句柄跨重启存活 | done | TODO-00 |
| TODO-19 | write 二进制安全 | done | TODO-14 |
| TODO-20 | 回归测试 + 文档 | done | 14..19 |

```
TODO-00 ─┬─> TODO-14 ──> TODO-19 ─┐
         ├─> TODO-15 ─────────────┤
         ├─> TODO-16 ─────────────┼─> TODO-20
         ├─> TODO-17 ─────────────┤
         └─> TODO-18 ─────────────┘
```

## 全局验收 gate
- TODO-00 + TODO-14..20 全 done。
- `cargo build --workspace` ✓ + `cargo build --workspace --features embed-fastembed` ✓
- `cargo clippy --workspace --all-targets -- -D warnings` ✓（零警告）
- `cargo test --workspace` = **45 passed**（默认；基线 35 + 新增 10）
- `cargo test --workspace --features embed-fastembed` = **49 passed**
- 量化：2000B 分 200 段写入后 DB 内容逐字节一致；`/index` getattr size == `cat | wc -c`；重启后旧句柄可读。

## 已锁定的开放决策
- **R1（/search、/where 大小）= 维持虚拟值**：动态内容不预计算，getattr 不触发检索；`/index` 报真实字节。
- **R2（写入缓冲落库时机）= 每次 WRITE 即 upsert 全文**：无 fsync/commit 钩点时的正确性优先策略。
- **R3（重启恢复完整性）= `/index` 可从 DB 推导；`/search`/`/where` 需重新 lookup**：标准 NFS 重连语义，已文档化。
