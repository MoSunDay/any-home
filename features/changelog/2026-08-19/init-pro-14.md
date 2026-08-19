# init-pro-14：本地挂载 e2e 补全 cargo 内置 rust 实现、首轮 TODO 状态回填（TODO-78..79）

## 背景

本地挂载 e2e 只有 `scripts/e2e.sh` 一条 bash/manual 路径，游离在
`cargo test` 之外：不可挂载的沙箱里核心环路零覆盖，rust 侧 222/234 的
测试矩阵从未真实碰过内核 NFSv3 客户端。另有首轮计划（init-pro）的
TODO-01..07 文档状态栏自 MVP 起一直欠着 `pending` 未回填。

## 变更

1. **rust 本地挂载 e2e（TODO-79）**：新增
   `crates/anyhome/tests/e2e_mount.rs`（395 行，单测试，纯函数无 class），
   为 `scripts/e2e.sh` 核心环路的 cargo 内置孪生——`CARGO_BIN_EXE_anyhome`
   起真实二进制（tempdir fs-root/db、自由端口 + EADDRINUSE 重试），
   `mount.nfs -o vers=3,tcp,nolock,noacl,port=P,mountport=P 127.0.0.1:/`
   真挂载；断言错误导出拒挂、真实文件 create/read/delete、/index→/search
   top-1、/where(+key 前缀)、index 与查询文件 true-size（无 NUL 填充）、
   覆写 + O_TRUNC 无旧尾、rm 三查（NFS 列目录 + /where/key: + 第二条
   libsql 连接直查 memories/memories_fts 行数归零）、unicode 百分号编码、
   /search_k2 封顶、SIGTERM 优雅停机（退出码 0 + 0 字节 WAL）后同库重启
   重挂的持久化。跳过语义与 e2e.sh 对齐：无 mount.nfs/非 root/非 Linux
   → 打印 skip 并通过；`ANYHOME_REQUIRE_MOUNT=1` 硬失败。分工不变：
   auth/agent facets 与 fastembed 全量 live gate 仍归 `scripts/e2e.sh`。
2. **首轮文档状态回填（TODO-78）**：`plans/init-pro/TODO-01..07.md`
   状态 pending→done + 证据回填（统一标注 2026-08-18 回填、技术内容
   零改动，证据链指回 MVP commit `169b627` 与后续 live 记录）。
3. **文档对账**：README 计数 222/234→223/235、tests 文件数与端到端
   小节补 rust e2e 一行；`plans/init-pro-14/` 全套记录。

## 不变量（未动）

`MemoryDb`/`MagicRegistry` 公共签名零改动（本次未新增 API）；魔法路径
语义与表/列/FTS 结构不变；`scripts/e2e.sh` 仍是全量 live parity gate
（rust e2e 只镜像其核心环路）；部署实例（pid 1077035、端口 11111、
release 二进制）零接触，回归全走临时端口 / 21115-21118。

## 验证

- `cargo build --workspace` 干净；`cargo test --workspace` = **223**
  passed / 0 failed（+1 即 e2e_mount，cargo test 内完成真实 mount）；
  `--features embed-fastembed` = **235** passed / 0 failed
  （fastembed_semantic 3 例无缓存自跳过，与之前一致）。
- clippy `--all-targets -- -D warnings` 默认与 embed-fastembed 两组合
  零警告。
- live：`scripts/e2e.sh --require-mount 21115` exit 0 全绿（含 auth
  facets，第二服务 21116）；rust e2e skip/require 双语义实弹验证
  （可执行退出 1 的 mount.nfs shim——0 字节非可执行 shim 因 shell 对
  EACCES 继续扫 PATH 而不构成遮蔽）。
- fastembed live e2e 本轮受阻：sandbox 出网 huggingface.co 与
  hf-mirror.com 均不可达、`scripts/fetch-model.sh` 停滞；改验 TODO-75
  fail-fast 守卫：`ANYHOME_EMBEDDER=fastembed scripts/e2e.sh` 立即
  exit 1 报模型缓存错误，不挂起。
