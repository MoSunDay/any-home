# TODO-36 · 显式耐久性 PRAGMA

**状态**：done

## 目标
让耐久性保证显式化（不依赖 bundled SQLite 编译期默认）并提升并发（读不阻塞写）。

## 变更
- `db/mod.rs::configure_durability()`（新）：设 `journal_mode=WAL` + `synchronous=FULL`，
  在 `open()` 中作为**第一步**调用（init_schema/migrate 等均在 WAL 下执行）。
- `journal_mode=WAL` 返回行 → 用 `query`（libsql `execute` 拒绝返回行的语句）。
- `db/mod.rs::durability()`（新，pub）：返回 `(journal_mode, synchronous)`，供测试/诊断。
- `:memory:` DB：WAL 不适用，PRAGMA 返回 `"memory"` 不报错（进程级易失语义不变）。

## 权衡
WAL + FULL = 并发读 + 满耐久（每次提交 fsync）。比 NORMAL 略慢但杜绝断电丢最后事务——
与「记忆服务不应丢已确认写」的产品定位一致。
