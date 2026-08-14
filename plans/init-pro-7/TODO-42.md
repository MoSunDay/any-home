# TODO-42 · `/search_k<N>`：top-k 可控

**状态**：done

## 目标
让 KNN 检索条数可由调用方按需指定，而不引入 CLI/SDK 面。

## 变更
- 新魔法路径族 `/search_k<N>/<query>` 与 `/agents/<id>/search_k<N>/<query>`：同 `/search`
  但 KNN top-k = N。
- `search_k` 后缀严格按数字解析，收敛到 1..=64（MAX_K）；`search_k0` / `search_k` /
  非数字后缀落回真实 FS（NOENT）。
- fileid 按 k 加盐：`/search_k8/q` 不与 `/search/q` 或其他 N 碰撞；registry 携带 k。
- readdir 不可枚举（与 DB 层查询面一致）；`/search` 原样（k=16，向后兼容）；无 CLI flag——
  k 由路径驱动。
