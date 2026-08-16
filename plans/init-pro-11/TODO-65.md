# TODO-65 · 本机再部署 + live e2e（运维步骤，无代码 commit）

**状态**：done

## 步骤与证据
1. 停止 init-pro-10 实例（pid 20630）；`cargo build --release`。
2. setsid 脱离会话拉起新常驻实例（127.0.0.1:11111，dummy embedder，dim 64）；
   启动日志 `anyhome::auth` policy assembly = Open——**证实平移后的函数**就位。
3. `scripts/e2e.sh --require-mount 21111` exit 0（真实 mount，非 skip 路径），
   含新增的错误 export 名拒绝负例。
4. 手工 mount 冒烟：index / where / key-prefix / search 全 OK；init-pro-10 的
   `k deployed smoke test ...` 记忆跨二进制替换存活——**write-through 耐久证据**。
5. `git status` 干净（scratch subagent 构建目录已清理）。

## 已知现象（记录不修，D6）
读取魔法文件会补 NUL 至 MAGIC_FILE_SIZE（1<<20）——magic.rs 既有行为（内核按
报告 size 零填充），真实内容后出现 ~1 MiB NUL 尾迹；候选未来 TODO，不影响
grep 型断言。
