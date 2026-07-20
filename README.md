# anyhome

面向 AI agent 的记忆系统。核心想法：把「记忆」做成一个本地 NFS 服务，agent 用最朴素的文件
操作（`ls` / `cat` / `echo` / `vim` ...）就能读写和检索记忆——无需任何专用 SDK 或 API。

## 为什么

AI agent 需要长期、可检索、能被多种工具共享的记忆。现有方案大多要求 agent 调用一套专有
API，结果是每个工具 / 框架都要重新对接一次，agent 也用不上自己本就擅长的「操作文件」能力。

NFS 几乎被所有操作系统原生支持、都能挂载。把记忆暴露成一个 localhost 上的 NFSv3 服务，
agent 用「读文件 / 写文件」这一最通用的能力即可访问记忆；shell、Python、Node、GUI 编辑器
全部零适配直接可用。

## 设计

按「能否被穷举、是否好按路径检索」，把记忆分两层：

### 1. 文件系统层 —— 存可穷举、好检索的数据
凡是天然呈树状、可枚举、能按路径定位的记忆，直接以文件 / 目录形式存在于 NFS 视图里。agent
可以 `ls` 浏览、`cat` 读取、`echo` 写入、`mkdir` 分类，就像管理一个真实目录。

> 例：`/agents/<id>/profile.md`、`/sessions/<id>/messages.log`、`/knowledge/<topic>/note.md`。

### 2. DB 层 —— 存不好检索的数据
凡是不可穷举、需要按语义 / 向量 / 全文检索的内容，放进数据库（基于内嵌的
[libSQL](./link_repos/libsql)，原生支持向量检索）。这部分对 agent 是「海量、不可枚举」的，
不适合摊成扁平文件。

### 3. 魔法路径 —— 用 NFS 路径做查询
把 DB 的检索能力伪装成文件系统的路径，agent 依然只需文件操作就能查 DB：

- **向量检索**：`cat /search/语义相关的笔记` —— 路径即 query，读到的「文件内容」就是 DB 的
  检索结果。
- **写入即索引**：`echo "待检索文本" > /index/<key>` —— 写入同时被向量化入库，供日后检索。
- **过滤 / 全文查询**：`cat /where/<词>` —— SQLite FTS5 全文检索（MATCH），返回命中记忆的
  `key\ttext` 行；`cat /where/key:<前缀>` 则按 key 前缀过滤（如 `cat /where/key:gree`
  命中所有 key 以 `gree` 开头的记忆）。

实现上对 NFS 协议做一次薄改：命中特定「查询目录」前缀的 `read` / `write` 不落盘，而是把路径
解析为查询、把结果作为文件内容返回。对 agent 而言，整个记忆系统始终是一棵统一的、可挂载的
目录树。

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

`link_repos/` 下的两个仓库是**只读内嵌依赖**（各自独立 git 仓库），不在本仓库内二次开发：

- [`link_repos/nfsserve`](./link_repos/nfsserve) —— 提供纯用户态 NFSv3 服务引擎，其
  `NFSFileSystem` trait 是本项目唯一的集成边界：实现一个自定义后端，把 `read` / `write` /
  `readdir` / `lookup` 等操作分流到「文件系统层」或「DB 层」。
- [`link_repos/libsql`](./link_repos/libsql) —— 提供 DB 与向量检索后端：原生 `vector('…')`
  类型 + `libsql_vector_idx(v)` KNN 索引。

## 使用

### 0. 准备依赖（只读内嵌）
```bash
scripts/bootstrap.sh        # 克隆并 pin link_repos/{nfsserve,libsql}
```

### 1. 构建
```bash
cargo build --workspace
```

### 2. 启动服务
```bash
./target/debug/anyhome \
    --fs-root ./anyhome-fs \
    --db-path ./memories.db \
    --addr 127.0.0.1:11111
```

> nfsserve 把 NFS / MOUNT / portmap 三个服务**复用在同一个端口**，且不跑 rpcbind、不实现 NLM。
> 因此挂载时必须显式指定 `vers=3,tcp,nolock,noacl,port=<端口>,mountport=<端口>`。

### 3. 挂载（需要 `mount.nfs` 与内核 NFS 客户端）
```bash
mkdir -p /mnt/anyhome
mount.nfs -o vers=3,tcp,nolock,noacl,port=11111,mountport=11111 localhost:/ /mnt/anyhome
```

### 4. 读写记忆（最朴素的文件操作）
```bash
# 写入即向量化入库
echo "hello anyhome" > /mnt/anyhome/index/greeting

# 路径即 query：读到的「文件内容」就是向量检索结果（key\ttext / 行）
cat /mnt/anyhome/search/hello%20anyhome
# => greeting	hello anyhome

# FTS5 全文检索：命中所有 text 包含该词的记忆
cat /mnt/anyhome/where/anyhome
# => greeting	hello anyhome

# 按 key 前缀过滤
cat /mnt/anyhome/where/key:gree
# => greeting	hello anyhome

# 文件系统层照常可用
mkdir -p /mnt/anyhome/notes && echo "普通文件" > /mnt/anyhome/notes/a.md
ls /mnt/anyhome            # index/  search/  where/  notes/  ...
```

### 5. 端到端冒烟
```bash
scripts/e2e.sh            # 自动构建→启动→挂载→index/search→断言→清理
```

> 默认 `/search` 由 `DummyEmbedder`（确定性、零依赖）驱动：相同文本 → 余弦相似度 1 → 命中 top-1。
> 真正的语义嵌入（local ONNX `Qdrant/all-MiniLM-L6-v2`，dim=384）已落地，但 feature-gated、默认关闭：
>
> ```bash
> # 一次性预取模型到标准 HF cache（离线 / 镜像环境用）
> scripts/fetch-model.sh          # 或 scripts/fetch-model.sh --mirror 走 hf-mirror.com
>
> # 带 fastembed feature 构建 + 以真实 embedder 启动
> cargo build --workspace --features embed-fastembed
> ./target/debug/anyhome --embedder fastembed \
>     --fs-root ./anyhome-fs --db-path ./memories.db --addr 127.0.0.1:11111
> ```
>
> 此时 `/search/语义近似的问法` 即可语义召回（10 条 paraphrase 集合上 recall@3 = 1.00，DummyEmbedder 仅 0.30）。

### 6. 持久化
记忆持久化在 SQLite/libSQL DB 文件（`--db-path`）里，进程重启后照常存活：

- 嵌入维度记录在 `meta` 表（首次打开写入），启动时校验；与历史数据不匹配即**硬错误**（拒绝启动），防止维度错配损坏索引。
- FTS5 全文索引随 DB 同表保存；若启动时索引为空（如外部重建后），自动 `rebuild` 填充。

## 状态

核心链路已打通并通过测试：

- `cargo test --workspace` = **35 用例全绿**（默认、零依赖）。
- `cargo test --workspace --features embed-fastembed` = **39 用例全绿**（模型可用时；模型缺失自动跳过 fastembed 相关测试）。

能力：

- 依赖内嵌：`link_repos/{nfsserve,libsql}` 经 `scripts/bootstrap.sh` 可复现 pin。
- NFS 后端分流：`AnyHomeFs`（dispatcher）把魔法路径导向 DB 层、其余路径导向文件系统层（`RealFs`）。
- 魔法路径：`/search/<q>`（向量 KNN）、`/index/<key>`（写入即索引）、`/where/<词>`（FTS5 全文 MATCH）/ `/where/key:<前缀>`（key 前缀过滤）。
- 向量检索：基于 libsql 原生 `FLOAT32(dim)` + `libsql_vector_idx`，`vector_top_k` 已验证可按余弦距离排序。
- 持久化：记忆写入 `--db-path` 指定的 DB 文件，重启存活；嵌入维度存于 `meta` 表、启动校验（不匹配=硬错误），FTS5 索引启动时按需自动重建。
- CLI：`anyhome --fs-root/--db-path/--addr/--export/--dim/--embedder`，单端口服务。

已解决的 init-pro TODO：

- **TODO-04（真实语义 embedder）= 已解锁**：默认 `DummyEmbedder` 零依赖、确定性；真实语义嵌入（本地 ONNX `Qdrant/all-MiniLM-L6-v2`，dim=384）由 `embed-fastembed` feature + `--embedder fastembed` 开启。10 条 paraphrase 集合上 recall@3 = 1.00（DummyEmbedder 仅 0.30），证明有效。模型不可用时相关测试 / 启动自动跳过。
- **TODO-05（`/where` 过滤查询）= done**：FTS5 全文 MATCH + `key:<前缀>` 过滤。
- **TODO-08（持久化 / 重启存活）= done**。
- **TODO-09（`/where` FTS5 过滤）= done**。
- **TODO-10（真实语义 embedder）= done**（feature-gated，模型不可用自动跳过）。
- **TODO-11（`--export` 接线 + mount）= done**。
- **TODO-12（真实挂载 e2e）= done**（代码层完成；实机验证需 `mount.nfs`，沙箱无此命令时 `scripts/e2e.sh` 自动跳过）。

> 注：init-pro 的 TODO-07（真实挂载 e2e）同 TODO-12——代码与脚本就绪，仅沙箱缺 `mount.nfs` / 内核 NFS 客户端时自动跳过。
