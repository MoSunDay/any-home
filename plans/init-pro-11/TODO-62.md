# TODO-62 · 查漏补缺：auth 合并语义测试 + e2e 卫生 + 静默 IO 日志

**状态**：done（commit db862bf）

## 范围决策
- `build_auth_policy` 位于 `main.rs`，lib 测试无法触达 → **纯逻辑平移**到
  `src/auth.rs`（`pub fn build_auth_policy(&Cli) -> anyhow::Result<AuthPolicy>`），
  逐字节相同：tracing 消息、`--agent-creds {path:?}: {e}` 错误包装不变；
  `main.rs` 改为调用 `anyhome::auth::build_auth_policy(&cli)?`（125 → 101 行）。
- `--export` MOUNT 强制的落点：强制逻辑在只读 nfsserve 依赖内，anyhome 无单元
  测试面 → 选 `with_export_name` 负例在 live e2e 里钉死（决策记录在案）。

## 变更
1. `src/auth.rs`：+7 例单元测试，补上契约空洞（该合并逻辑此前**零测试**）：
   flag-only Enforced、creds-file-only、flag∩file 重复绑定按集合语义去重、
   并集合并、空配置 → Open 回退、缺文件 → Err 含路径、坏文件（uid 0 被拒）→
   Err 含路径。
2. `scripts/e2e.sh`：(i) `--export` MOUNT 强制补**负例**——挂错误 export 名被拒
   （MNT3ERR_NOENT）；(ii) 5 处字面 TAB grep 模式改用 `${T}`（脆弱性修复）；
   PASS 行同步更新。
3. `src/dispatcher/dbops.rs`：6 处静默 `NFS3ERR_IO` 映射补
   `tracing::warn!(op=..., error=%e)`（search:embed / search:query / index:embed /
   index:upsert / where:query / index_text:get_text）；返回值与控制流零变化。

## 验证
- commit db862bf diff：main.rs -28 行、auth.rs +136 行（函数 + 7 例测试）、
  dbops.rs +30/-、e2e.sh +20/-。
- 全量回归见 index.md 计数对账（workspace 203 / fastembed 207）。
