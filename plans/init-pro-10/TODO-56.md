# TODO-56 · 查漏补缺：文档 off-by-one + gitignore 部署产物

**状态**：done（commit 672972f）

## 变更
- `plans/init-pro-9/TODO-51.md:14`：`readdir.rs +55（152→207）` → `+54（152→206）`。
  取证：`git diff ccafc0c^ ccafc0c --stat` 显示 readdir.rs +54；`git show
  ccafc0c^:...readdir.rs | wc -l` = 152，现行 206 行。
- `.gitignore` 追加 `/anyhome-fs/`、`/anyhome.db*`、`/anyhome-server.log`。

## 验证
- TODO-60 部署后 `git status --short` 空输出（产物全部被忽略）。
