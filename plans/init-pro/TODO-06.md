# TODO-06 二进制 + CLI 配置

## 目标
`anyhome` 命令按参数启动分流型 NFS 服务，把各模块拼成可交付入口。

## 核心实现
- `Cargo.toml` 加 `[[bin]] name="anyhome"` + `clap={workspace=true,features=["derive"]}`。
- `src/main.rs`：`Cli{fs_root:PathBuf, db_path:PathBuf, addr:String, export:String, dim:usize}`，
  合理默认（addr=`127.0.0.1:11111`，export=`/`，dim=DummyEmbedder 的值）。
- 组装：MemoryDb::open + Embedder（按 TODO-04 状态选；**未决策则默认 DummyEmbedder + warn 日志**）
  + RealFs::new -> AnyHomeFs -> `NFSTcpListener::bind(&addr, fs).await` -> `handle_forever().await`。
- tracing_subscriber::fmt init。

## 验收手段
- `--help` 列全参数
- `Cli::try_parse_from(["anyhome","--fs_root","/tmp/x",...])` 单测
- 启动后 `ss -ltnp | grep 11111` 确认端口
- 手动 mount（用 TODO-07 的 mount 命令）成功

## 状态
pending

## 证据
（完成后填）

## 卡点
- Embedder 选择依赖 TODO-04；未决策前 main 默认 DummyEmbedder 并 warn。

## 依赖
TODO-05
