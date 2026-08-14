# init-pro-7：检索可控性 + 鉴权泄漏收口（TODO-40..45）

## 背景

init-pro-5 交付了 agent 鉴权（lookup/read/write 均按 `require_agent` 拦截），但 **TODO-40 spike
在代码级证实了两个遗留缺口**：

- **大小 oracle**：`lookup` 会在**已授权**调用方名下向进程级 MagicRegistry 注册 `MagicEntry`，
  而 fileid 可离线推导（`"{agent}\0{key}"` 的 FNV-1a + CAT_INDEX 偏移）——持有/猜中 fileid 的
  **未授权**调用方可以直接 `getattr` 读到真实存储字节数。这违背 init-pro-5「泄漏句柄已收敛」的语义。
- **旧注释错误**：`/where` 一侧的旧代码注释称「畸形 FTS 查询 → 无行返回」。实测推翻：畸形 FTS5
  MATCH 表达式在 fetch 时抛出 libsql `SqliteFailure`（`fts5: syntax error near ...`、
  `unterminated string`、`unknown special query: ...`），经 NFS 层表现为不可读的 `IO` 错误。

第三个缺口来自检索可控性：`/search` 的 KNN top-k 固定为默认值 16，调用方无法按需调节。

spike 产出**修正后的鉴权矩阵**（见 plans/init-pro-7/index.md），驱动本迭代修复。

## 真实缺口与修复

1. **TODO-41 大小 oracle 关闭**：`dispatcher.rs` 的 `getattr`/`setattr` 对 agent 范围的
   CAT_INDEX fileid 调用 `require_agent`，未授权返回 `NFS3ERR_ACCES`。目录、search/where
   虚拟文件与真实 FS 元数据**不拦截**（init-pro-5 D4 决策由「getattr/setattr 从不鉴权」收窄为
   「仅目录与真实 FS 元数据不拦截」）。默认 agent（`/index`，`agent_id = ""`）保持公开；
   无法解析的陈旧 CAT_INDEX fileid 维持零大小属性（init-pro-6 重启语义不变）。Open 策略下
   （未配置凭证）行为完全不变。
2. **TODO-42 `/search_k<N>`**：新增魔法路径族 `/search_k<N>/<query>` 与
   `/agents/<id>/search_k<N>/<query>`——同 `/search` 但 KNN top-k = N。`search_k` 后缀严格
   按数字解析，收敛到 1..=64（MAX_K）；`search_k0` / `search_k` / 非数字后缀落回真实 FS
   （NOENT）。fileid 按 k 加盐，`/search_k8/q` 不会与 `/search/q` 或其他 N 碰撞。readdir
   不可枚举（与 DB 层查询面一致）。`/search` 原样保留（k=16，向后兼容），无 CLI flag——k 由
   路径驱动。
3. **TODO-43 `/where` 错误可读**：db 层 `where_search` 改为返回 `WhereOutcome` 枚举：
   `Rows(Vec<(key,text)>)` 或 `InvalidFts(cond)`（按 SQLite 错误消息分类畸形 FTS5 MATCH；
   参数绑定不变——注入安全性保持）。dispatcher 把 `InvalidFts` 渲染为文件首行
   `# error: invalid FTS query: <cond>\n`——NFS READ 没有 errno 通道，只能以文本承载。
   合法查询输出与之前逐字节一致。

## 关键决策（不做什么）

- **不做强 auth**（TLS/mTLS、RPCSEC_GSS/Kerberos）：soft auth 定位不变——防误操作与意外串扰，
  非对抗性安全；维持既有锁定。
- **不做 uid 0 绑定**：AUTH_SYS 下 uid 0 语义不可区分（匿名 vs root），维持拒绝绑定。
- **不做 RealFs chown**：文件系统层的属主模型与 agent 鉴权解耦，不引入跨层耦合。
- **不做写放大优化**：本迭代不触碰写路径，避免与鉴权/检索改动相互干扰。

## 变更文件

- `crates/anyhome/src/magic.rs`：`search_k<N>` 路径解析、k-salt fileid、registry 携带 k。
- `crates/anyhome/src/db/mod.rs`：`WhereOutcome` 枚举 + FTS 错误分类。
- `crates/anyhome/src/dispatcher.rs`：`getattr`/`setattr` 鉴权拦截 + `/where` 错误行渲染。
- `crates/anyhome/tests/auth_metadata.rs`（新，7 用例）。
- `crates/anyhome/tests/where_fts.rs`（+3）/ `crates/anyhome/tests/anyhome_magic.rs`（+5）。
- `README.md` / `README_EN.md`：能力说明、已知限制收窄、测试计数。

## 验收

- `cargo build --workspace` ✓（默认 + `--features embed-fastembed`）。
- `cargo clippy --workspace --all-targets -- -D warnings` ✓ 零警告。
- `cargo test --workspace` = **102 passed**（87 既有 + 7 auth_metadata + 3 where_fts +
  5 anyhome_magic）；`--features embed-fastembed` = **106 passed**。既有用例**零回归**
  （鉴权拦截仅在 Enforced 策略 + agent 范围 fileid 上生效；`/search` 与合法 `/where` 输出不变）。
- 行数：`dispatcher.rs` 760 / `magic.rs` 356 / `db/mod.rs` 650（迭代中，均 <800）。

## Related Docs
- [plans/init-pro-7/index.md](../../../plans/init-pro-7/index.md)
- [既有 changelog：init-pro-6](../2026-08-04/init-pro-6.md)
