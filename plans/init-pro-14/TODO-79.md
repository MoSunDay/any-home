# TODO-79 · 本地挂载 e2e 补全 rust 实现 + 全量回归

## 背景

本地挂载 e2e 此前只有 `scripts/e2e.sh` 一条路：bash、manual、游离在
`cargo test` 之外——不可挂载的沙箱里核心环路完全不跑，rust 侧测试矩阵
（222/234）从未真实碰过内核 NFSv3 客户端。本 TODO 把 e2e.sh 的核心
本地挂载环路做成 cargo 内置的 rust 孪生。

## 产出

- `crates/anyhome/tests/e2e_mount.rs` 新文件（395 行，单
  `#[test] fn local_mount_e2e`，纯函数 + 常量、无 class，遵守仓库纯函数式
  规则）：`scripts/e2e.sh` 核心本地挂载环路的 cargo 内置孪生——
  - `env!("CARGO_BIN_EXE_anyhome")` 拉起**真实 anyhome 二进制**
    （tempdir 作 fs-root / db-path），自由端口选端口 + EADDRINUSE 重试；
  - `mount.nfs -o vers=3,tcp,nolock,noacl,port=P,mountport=P
    127.0.0.1:/` 真内核 NFSv3 挂载（挂载参数与 e2e.sh 逐字一致）；
  - 断言面（全部在真实挂载上过）：
    MOUNT 错误导出名拒挂；真实文件 create/read/delete 生命周期
    （fs-root 物化）；`/index`→`/search` top-1 精确命中；`/where` FTS
    双 key 同现；`/where/key:<prefix>` 前缀过滤；index 文件与查询文件
    的 true attr size（< 1 MiB，无 NUL 填充）；覆写 + O_TRUNC
    （`File::create` == shell `>`）不留旧尾；rm 三查 = NFS 列目录 +
    `/where/key:` + 第二条 libsql 连接直查 DB/FTS 行数
    （`memories` where `agent_id='' and key='delete-me'` == 0、
    `memories_fts MATCH 'temp'` == 0）；unicode 百分号编码的 `/search`
    与 `/where`；`/search_k2` 行数封顶；SIGTERM 优雅停机（退出码 0 +
    checkpoint TRUNCATE 后 0 字节 WAL）→ 同 db-path 重启重挂 →
    持久化断言（`overwrite-key`=="short\n"、`/where/hello` 含
    greeting+second、已删 key 不复活）。
- **跳过语义与 e2e.sh 对齐**：无 mount.nfs / 非 root / 非 Linux → 打印
  `skipped: ...` 后通过（NFS-less 沙箱 CI 保持绿）；
  `ANYHOME_REQUIRE_MOUNT=1` 变硬失败。两条路径均实弹验证（可执行、
  退出 1 的 mount.nfs shim 前置 PATH，见坑 a）。
- **分工（文档化）**：rust = cargo test 内的核心环路；auth/agent facets
  与 fastembed 的全量 live 对齐 gate 仍归 `scripts/e2e.sh`。
- 文档：`plans/init-pro-14/`（本文件 + index）、
  `features/changelog/2026-08-19/init-pro-14.md`、README 计数
  222/234→223/235 与 e2e 小节一行指引。

## 设计与坑（实弹记录）

- **坑 a · 非可执行 shim 不遮蔽 mount.nfs**：想模拟"主机无 mount.nfs"
  验证 skip 语义时，放一个 0 字节不可执行文件在 PATH 前面**无效**——
  shell 对 `EACCES` 会继续扫后续 PATH 条目，最终仍命中真 mount.nfs。
  必须用**可执行且退出非 0** 的 shim 才构成有效遮蔽。
- **坑 b · libsql 连接两层 Result**：`Builder::new_local(path).build()`
  是 async（`.await` 得第一个 Result），`handle.connect()` 又是一层
  `Result`——两层都要 `map_err` 进测试的 `String` 错误流；取标量用
  `conn.query(sql, ()).await` + `rows.next().await` + `row.get::<i64>(0)`。
- **坑 c · 自由端口竞态**：bind(0) 拿端口后关 listener 再让 server 绑定，
  中间窗口端口可能被抢（EADDRINUSE）。解法：重试环——读 server 日志
  确认 `Address already in use` 才换端口重来，最多 3 次；不能只看
  connect 探活失败就换端口（会误伤慢启动）。
- **坑 d · 清理顺序**：先 umount、再 kill server、最后才让 TempDir
  drop——挂载还活着时删 tempdir 会失败（EBUSY）。`umount -f` 失败回退
  `-l`；kill 走 SIGTERM + 限时等待、超时 SIGKILL 兜底；清理在
  `flow` 返回后无条件执行（断言失败也要卸载）。
- **坑 e · `CARGO_BIN_EXE_anyhome`**：集成测试里 `env!` 该变量会让
  cargo 自动构建 bin，拿到可直接 spawn 的二进制路径——无需手工拼
  target/ 路径或预构建步骤。

## 验收手段与证据（2026-08-19，全在本机）

- `cargo build --workspace`：干净。
- `cargo test --workspace` = **223** passed / 0 failed（原 222；+1 即
  本测试，在 cargo test 里完成了**真实 mount**）。
- `cargo test --workspace --features embed-fastembed` = **235** passed /
  0 failed（原 234；`fastembed_semantic` 3 例无模型缓存自跳过，与之前
  一致）。
- `cargo clippy --workspace --all-targets -- -D warnings`：默认与
  embed-fastembed 两组合均 0 警告。
- `scripts/e2e.sh --require-mount 21115`：exit 0，全量 live parity 绿
  （含 auth facets，第二服务 21116）。
- rust e2e 的 skip / require 双语义实弹验证（shim 法，见坑 a）。
- fastembed live e2e 本轮受阻：sandbox 出网 huggingface.co 与
  hf-mirror.com 均不可达，`scripts/fetch-model.sh` 拉取停滞（无模型
  缓存）；改验 TODO-75 fail-fast 守卫：`ANYHOME_EMBEDDER=fastembed
  scripts/e2e.sh` 立即 exit 1 并报模型缓存错误，不挂起。
- 部署实例（pid 1077035、端口 11111、release 二进制）零接触；全部回归
  走临时端口 / 21115-21118。

## 状态

done
