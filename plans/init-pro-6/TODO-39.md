# TODO-39 · 文档更新

**状态**：done

## 变更
- `README.md` / `README_EN.md`「已知限制」：删除错误的「写入缓冲丢失」描述，
  改为准确的「写入耐久性（init-pro-6 已硬化）」write-through + WAL + FULL 说明。
- 「状态」能力清单：持久化条目补充耐久性说明。
- 测试计数：82 → 85（默认）/ 86 → 89（fastembed）。
