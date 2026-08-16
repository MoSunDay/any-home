# TODO-71 · 文档三件套 + 计数对账

## 产出

- `plans/init-pro-12/`：`index.md` + TODO-67..71 五份记录（本文件在内）。
- `features/changelog/2026-08-16/init-pro-12.md`。
- `README.md` 测试计数更新（运行测试小节 + 状态小节）。

## 计数对账（相对 init-pro-11 收尾基线 203/207）

| 变化 | 归属 | 数量 |
|------|------|------|
| `magic_write_semantics` 截断套件 +4（含 1 处旧钉翻转，净值 +4） | TODO-70 | +4 |
| `magic.rs` truncate_buffer 内联单测 +3 | TODO-70 | +3 |
| `tests/magic_mkdir_guards.rs` 新文件 | TODO-68 | +7 |
| `db_vector` 饥饿回归例 +1 | TODO-68 | +1 |
| `libsql_vector_semantics` k0 子进程探针对（2）→ 进程内 k0 断言（1） | TODO-68 | −1 |

203 + 4 + 3 + 7 + 1 − 1 = **217**（`cargo test --workspace` 实测，0 failed）。
fastembed：217 + 4（`fastembed_semantic` 仅 feature 开启时参跑）= **221**
（实测一致）。收尾套件分布：lib 47、集成 21 个文件；最大单文件
`libsql_fts_query_syntax` 21 例。

## 验收口径

- `cargo test --workspace` = 217 passed / 0 failed；`--features
  embed-fastembed` = 221 passed / 0 failed。
- `cargo clippy -p anyhome --all-targets` 零警告（修复了
  `magic_mkdir_guards` 一处 `bool_comparison`）。
- `scripts/e2e.sh --require-mount 21111` exit 0：6 facets + 重启持久化 +
  auth + export 强制。
- TODO-69：phase A/B 全过，部署记忆字节零变化，regress-12-* 三重零残留。
