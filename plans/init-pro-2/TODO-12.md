# TODO-12 真实挂载 e2e

## 目标
`scripts/e2e.sh` 覆盖完整 build→start→mount→index/search/where→assert→cleanup 链路；
`--require-mount` 模式做硬 gate。

## 核心实现
- e2e.sh 支持 `--require-mount`：mount.nfs 缺失时 exit 1 而非 exit 0。
- `ANYHOME_EMBEDDER` env 透传 `--embedder`。
- 新增 `/where` 断言。
- mount.nfs 缺失时默认自动跳过（exit 0）。

## 验收手段
- 沙箱有 mount.nfs 时跑通全链路。
- 无 mount.nfs 时自动跳过（exit 0）。
- `--require-mount` 时 exit 1。

## 状态
done（沙箱无 mount.nfs，自动跳过；TODO-12 无法在此沙箱 live-verify，但代码 + 脚本就绪）

## 证据
- `bash -n scripts/e2e.sh` 语法检查通过。
- dispatcher 级 e2e（`anyhome_magic.rs`）9/9 passed，覆盖逻辑正确性。

## 卡点
- 沙箱无 mount.nfs / 内核 NFS 客户端，无法 live 验证真实挂载（与 init-pro TODO-07 相同约束）。

## 依赖
TODO-11。
