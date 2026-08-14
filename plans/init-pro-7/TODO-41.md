# TODO-41 · 关闭大小 oracle（metadata 鉴权）

**状态**：done

## 目标
让 `/index` 文件的真实字节数不再对未授权调用方可读，兑现 init-pro-5「泄漏句柄已收敛」语义。

## 变更
- `dispatcher.rs`：`getattr`/`setattr` 对 agent 范围 CAT_INDEX 文件的 fileid 调用
  `require_agent`，未授权 → `NFS3ERR_ACCES`。
- 不拦截：目录、search/where 虚拟文件、真实 FS 元数据（D4 收窄为「仅目录与真实 FS 元数据
  不拦截」）；默认 agent（`/index`，`agent_id = ""`）保持公开。
- 陈旧（不可解析）CAT_INDEX fileid 维持零大小属性——init-pro-6 重启语义不变。
- Open 策略（未配置凭证）下行为完全不变。
