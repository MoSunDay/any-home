# TODO-44 · 验证测试

**状态**：done

## 变更
- `tests/auth_metadata.rs`（新，7 用例）：泄漏句柄 oracle 关闭（含匿名 uid 0）、setattr 拦截、
  默认 agent 公开、目录/search/where/真实 FS 元数据不受影响、陈旧 fileid 零大小形态、
  Open 策略向后兼容、agent 范围 search_k + `..` 父目录解析。
- `tests/where_fts.rs`（+3）：畸形 → `InvalidFts` 携带原条件、无命中仍为 `Rows`、
  `key:` 前缀含 FTS 元字符。
- `tests/anyhome_magic.rs`（+5）：search_k 截断结果、收敛/拒绝/不可枚举、fileid 无碰撞、
  含斜杠 query 走百分号编码、`/where` 错误行渲染。
- 全部非 `#[ignore]`；测试总数 87/91 → **102/106**，零回归。
