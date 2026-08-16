[English](README_EN.md) | [中文](README.md)

# anyhome

[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](LICENSE)

**面向 AI agent 的可挂载 NFSv3 记忆服务：把「记忆」变成一个本地文件系统。**

## 为什么

anyhome 把「记忆」做成一个本地 NFS 服务。AI agent 用最朴素的文件操作
（`ls` / `cat` / `echo` / `vim` …）就能读写和检索记忆——无需任何专用 SDK，无需任何 API。

NFS 几乎被所有操作系统原生支持、都能挂载。把记忆暴露成 `localhost` 上的 NFSv3 服务，
agent 用「读文件 / 写文件」这一最通用的能力即可访问记忆；shell、Python、Node、GUI
编辑器全部零适配直接可用。

## 设计

按「能否被穷举、是否好按路径检索」，把记忆分两层，再用「魔法路径」把两层连通：

### 1. 文件系统层 —— 存可穷举、好检索的数据

凡是天然呈树状、可枚举、能按路径定位的记忆，直接以文件 / 目录形式存在于 NFS 视图里。
agent 可以 `ls` 浏览、`cat` 读取、`echo` 写入、`mkdir` 分类，就像管理一个真实目录。

> 例：`/agents/<id>/profile.md`、`/sessions/<id>/messages.log`、`/knowledge/<topic>/note.md`。

### 2. DB 层 —— 存不好检索的数据

凡是不可穷举、需要按语义 / 向量 / 全文检索的内容，放进数据库（基于内嵌的 libSQL ——
一个原生支持向量检索的 SQLite 衍生分支）。这部分对 agent 是「海量、不可枚举」的，
不适合摊成扁平文件。

### 3. 魔法路径 —— 用 NFS 路径做查询

把 DB 的检索能力伪装成文件系统的路径，agent 依然只需文件操作就能查 DB：

- **向量检索**：`cat /search/<query>` —— 向量 KNN 检索，路径即 query，文件内容为结果
  （每行 `key<TAB>text`）。
- **检索条数可控**：`cat /search_k<N>/<query>` —— 同 `/search` 但显式指定 KNN top-k
  （N 收敛到 1..=64；`/search` 默认 16）。
- **写入索引**：`echo "text" > /index/<key>` —— 写入即嵌入并 upsert 进向量库。
- **全文检索**：`cat /where/<word>` —— FTS5 全文 MATCH 检索。
- **键前缀过滤**：`cat /where/key:<prefix>` —— 按 key 前缀过滤。
- **畸形 FTS 查询**：`cat /where/<cond>` 语法错误时返回首行
  `# error: invalid FTS query: <cond>`（合法查询输出不变）。

### 多 agent 命名空间隔离

每个 agent 拥有独立的魔法路径命名空间：

- `/agents/<id>/search/<q>` —— 仅在 agent `<id>` 范围内做 KNN
- `/agents/<id>/search_k<N>/<q>` —— agent 范围内 KNN，top-k = N
- `/agents/<id>/index/<key>` —— 写入归入 agent `<id>`
- `/agents/<id>/where/<cond>` —— 仅在 agent `<id>` 范围内做 FTS / 前缀
- 首次写入即隐式创建 agent
- 全局 `/search`、`/index`、`/where` 映射到默认 agent（`agent_id = ""`），保持向后兼容

### Agent 身份鉴权（soft auth）

可选。注册 agent 凭证，把 `agent_id` 绑定到调用方 NFS `AUTH_SYS` 的 `uid`。开启后，
调用方必须拥有匹配的 `uid` 才能访问 `/agents/<id>/...`；冒充访问返回 `NFS3ERR_ACCES`；
未授权的 agent 会在 `/agents` 列表中被隐藏。默认 agent（全局 `/{search,index,where}`）
始终公开。当未配置任何凭证时，鉴权关闭（`Open` 策略），完全向后兼容。

## 架构

```
                  +------------- agent (shell / editor / script) -------------+
                  |        ls / cat / echo  --  any file op                    |
                  +------------------------------+----------------------------+
                                                 |  mount localhost:NFSv3
                  +------------------------------v----------------------------+
                  |  anyhome NFSFileSystem impl (dispatcher)                   |
                  |   - normal path  -> filesystem layer (enumerable memory)   |
                  |   - magic path   -> query parse -> DB layer (vector/full)  |
                  +----------+--------------------------------+---------------+
                             |                                |
              +--------------v-----------+    +---------------v---------------+
              | link_repos/nfsserve      |    | link_repos/libsql             |
              | user-mode NFSv3 engine   |    | SQLite fork, native vectors   |
              | NFSFileSystem = boundary |    | vector() / libsql_vector_idx  |
              +--------------------------+    +-------------------------------+
```

## 快速开始

### 0. 准备只读内嵌依赖

`link_repos/{nfsserve,libsql}` 是钉死的、只读的内嵌依赖，已被 gitignore。用脚本可复现地引导：

```bash
scripts/bootstrap.sh
```

### 1. 构建

```bash
cargo build          # 默认：dummy embedder，零网络依赖
cargo build --features embed-fastembed   # 真实语义嵌入（ONNX all-MiniLM-L6-v2，dim 384）
```

### 2. 启动服务

```bash
cargo run -- --fs-root ./anyhome-fs --db-path ./anyhome.db --addr 127.0.0.1:11111
# 带语义检索：
cargo run --features embed-fastembed -- --embedder fastembed --db-path ./anyhome.db
# 带 agent 鉴权：
cargo run -- --agent-uid botA:1001 --agent-uid botB:1002 --db-path ./anyhome.db
# 或从凭证文件加载：
cargo run -- --agent-creds /etc/anyhome/agents.conf
```

### 3. 挂载（需要 `mount.nfs` 与内核 NFS 客户端）

```bash
mkdir -p /mnt/anyhome
mount -t nfs -o port=11111,mountport=11111,nfsvers=3,nolock 127.0.0.1:/ /mnt/anyhome
```

### 4. 用纯文件操作读写记忆

```bash
# 写入 = 嵌入 + upsert 进向量库
echo "hello anyhome" > /mnt/anyhome/index/greeting

# 路径即 query：读取返回向量 KNN 结果（每行 key<TAB>text）
cat /mnt/anyhome/search/hello
# => greeting	hello anyhome

# 同上，但显式指定 KNN top-k
cat /mnt/anyhome/search_k4/hello   # 同上，但只返回 top-4

# FTS5 全文检索
cat /mnt/anyhome/where/hello
# => greeting	hello anyhome

# key 前缀过滤
cat /mnt/anyhome/where/key:gre

# 文件系统层照常工作
echo "# my notes" > /mnt/anyhome/notes/readme.md
```

### 5. 端到端冒烟测试

```bash
scripts/e2e.sh                          # 无 mount.nfs 则跳过
scripts/e2e.sh --require-mount          # 无 mount.nfs 则硬失败
ANYHOME_EMBEDDER=fastembed scripts/e2e.sh  # 带语义嵌入
```

### 6. 运行测试

```bash
cargo test                              # 203 个测试（默认，零依赖）
cargo test --features embed-fastembed   # 207 个测试
cargo clippy --all-targets -- -D warnings
```

## CLI 参考

```
anyhome [OPTIONS]

OPTIONS:
    --fs-root <PATH>           支撑文件系统层的磁盘目录（默认 ./anyhome-fs）
    --db-path <PATH>           libSQL 数据库路径，":memory:" 为临时库（默认 :memory:）
    --addr <IP:PORT>           监听地址，单端口多路复用（默认 127.0.0.1:11111）
    --export <PATH>            暴露给 NFS 客户端的导出路径（默认 /）
    --embedder <KIND>          dummy（默认）或 fastembed（需 --features embed-fastembed）
    --dim <N>                  dummy embedder 的向量维度（默认 64；fastembed 固定 384）
    --agent-uid <ID:UID>       把 agent_id 绑定到调用方 AUTH_SYS uid（可重复）。开启鉴权。
    --agent-creds <FILE>       凭证文件，每行一个 id:uid，支持 # 注释。开启鉴权。
```

凭证文件格式（`--agent-creds`）：

```
# agent credentials
botA:1001
botB:1002
```

## 嵌入后端

| 后端 | Feature | 模型 | `/search` 行为 | 网络 |
|------|---------|------|----------------|------|
| `dummy`（默认） | — | 确定性哈希 | 仅精确匹配 | 无 |
| `fastembed` | `embed-fastembed` | all-MiniLM-L6-v2（dim 384，本地 ONNX） | 真实语义 KNN | 模型仅获取一次（或用 `scripts/fetch-model.sh` 预取） |

## 状态

核心管线已构建并通过完整测试：

- `cargo test` = **203 个测试通过**（默认，零依赖）
- `cargo test --features embed-fastembed` = **207 个测试通过**
- `cargo clippy --all-targets -- -D warnings` = **0 警告**（两种 feature 组合）

能力清单：

- **只读内嵌依赖**：`link_repos/{nfsserve,libsql}` 已钉版本，通过 `scripts/bootstrap.sh` 可复现构建
- **NFS 后端分发**：`AnyHomeFs` 把魔法路径路由到 DB 层，其余路由到文件系统层（`RealFs`）
- **魔法路径**：`/search/<q>`（向量 KNN）、`/search_k<N>/<q>`（同 `/search`，KNN top-k = N，
  收敛到 1..=64）、`/index/<key>`（写入 = 嵌入 + upsert）、
  `/where/<word>`（FTS5 MATCH）/ `/where/key:<prefix>`（key 前缀过滤）。`/search`、
  `/search_k` 与 `/where` 是只读查询视图（写入 / setattr 返回 `ROFS`）
- **多 agent 命名空间隔离**：`/agents/<id>/{search,index,where}`，拥有独立的 DB 键空间与检索范围
- **Agent 身份鉴权（soft auth）**：可选的 `--agent-uid` / `--agent-creds`；AUTH_SYS uid 绑定
- **向量检索**：libsql 原生 `FLOAT32(dim)` + `libsql_vector_idx`，余弦距离 KNN
- **持久化与写入耐久性**：写入 `--db-path` 的记忆可跨重启保留；嵌入维度存于 `meta` 表，启动时校验；
  FTS5 索引按需在启动时自动重建。每次 `WRITE` 即写穿 DB（write-through），文件型 DB 用
  `WAL` + `synchronous=FULL`，已提交写 fsync 后存活，崩溃不丢数据（init-pro-6）
- **CLI**：单端口服务，配置完备

## 已知限制

- **写入耐久性（init-pro-6 已硬化）**：每次 `WRITE` RPC **立即** upsert 到 DB（write-through，非缓冲至 close）。
  文件型 DB 使用 `WAL` + `synchronous=FULL`，每个已提交写均 fsync、崩溃后存活；内存写缓冲仅用于偏移拼接，
  双重上限：256 条目 + 总计 8 MiB（超出按最久未写优先淘汰；单缓冲自身超限时组合后丢弃，
  淘汰安全——DB 始终为权威来源，下次写从 DB re-seed；init-pro-8）。
- **仅 soft auth**：NFSv3 AUTH_SYS 的 `uid`/`gid` 由客户端自报、**极易伪造**——本机制仅防
  误操作与意外串扰，**不构成对抗性安全**。`/index` 文件的元数据（含真实字节大小）自
  init-pro-7 起随 agent 鉴权（未授权 `getattr`/`setattr` 返回 `ACCES`，关闭「大小 oracle」）；
  **目录与真实文件系统元数据不做鉴权**（对 NFS 客户端属性缓存友好；沙箱无 mount.nfs，
  真实挂载下的客户端缓存交互未做 live 回归）。
  如需强鉴权，应在 TLS/mTLS 终止的网络前置，或升级至 RPCSEC_GSS/Kerberos。
- **root（uid 0）无法安全使用 agent 鉴权**：AUTH_SYS 下 `uid 0` 同时表示「匿名调用方」（AUTH_NULL）与「真实 root」，二者不可区分。
  因此 `--agent-uid` / `--agent-creds` 会**拒绝** `uid 0` 的绑定——若允许，所有匿名调用方将获得该 agent 的访问权。agent 进程须以非 root uid 运行。

## 项目结构

```
anyhome/
├── crates/anyhome/
│   ├── src/
│   │   ├── lib.rs          # library crate 根
│   │   ├── main.rs         # 二进制入口，构建 AnyHomeFs
│   │   ├── cli.rs          # clap CLI 定义
│   │   ├── dispatcher.rs   # AnyHomeFs：NFSFileSystem 分发（真实 fs + 魔法路径）
│   │   ├── dispatcher/     # AnyHomeFs inherent impl 子模块（dbops/routing/readdir）
│   │   ├── auth.rs         # AuthPolicy、CallerCred、AgentCred（soft auth）
│   │   ├── magic.rs        # 魔法路径编码、fileid 分类、MagicRegistry
│   │   ├── backend/        # RealFs：磁盘文件系统层
│   │   └── db/             # MemoryDb（libsql）：schema/crud/query 子模块 + embedder
│   └── tests/              # 集成测试（20 个文件）
├── link_repos/             # 只读内嵌依赖（gitignored）
│   ├── nfsserve/           # 用户态 NFSv3 引擎（含 anyhome fork 补丁）
│   └── libsql/             # 原生支持向量检索的 SQLite 衍生分支
├── scripts/
│   ├── bootstrap.sh        # 可复现的依赖准备 + 补丁应用
│   ├── e2e.sh              # 端到端 NFS 挂载冒烟测试
│   ├── fetch-model.sh      # 预取 fastembed 模型以便离线使用
│   └── patches/            # 在钉死的上游之上应用的本地 fork 补丁
├── features/changelog/     # 迭代 changelog（设计历史）
├── Cargo.toml              # workspace 根
└── rust-toolchain.toml     # MSRV 钉版本（1.85.0）
```

实现说明：

- `link_repos/*` 是只读内嵌依赖（独立 git 仓库，已 gitignore），**不是** workspace 成员。
  nfsserve 携带一个小型本地 fork 补丁（`scripts/patches/nfsserve-auth-identity.patch`），
  由 `scripts/bootstrap.sh` 应用，用以把 AUTH_SYS 调用方身份暴露给 `NFSFileSystem` trait。
- nfsserve 的 `NFSFileSystem` trait 是唯一的集成边界。

## 依赖与构建要求

- Rust 1.85.0（通过 `rust-toolchain.toml` 钉版本）
- 带有 NFS 客户端支持（`mount.nfs`）的 Linux，用于挂载
- git + 网络访问，用于首次运行 `scripts/bootstrap.sh`
