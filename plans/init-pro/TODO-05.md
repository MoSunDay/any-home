# TODO-05 魔法路径路由（分流 + DB 查询路径）

## 目标
`AnyHomeFs` 成分流器：普通路径->真实 FS，魔法前缀->DB。
`cat /search/<query>` 返 KNN 结果，`echo "text">/index/<key>` 向量化入库。

## 核心实现
- `AnyHomeFs{real: RealFs, db: MemoryDb, embedder: Arc<dyn Embedder>}`。
- fileid：真实 FS 用低位 inode；魔法用 `0x8000_0000_0000_0000 | inner_key`（inner_key 分段：
  目录标识 + 查询/键哈希）。
- **R4=可见**：根 `readdir` 拼入三个魔法目录条目（`/search` `/index` `/where`，
  fileid = `0x8000...|{DIR_TAG}`），让 `ls /mnt` 可见。魔法目录自身的 readdir 返确定性条目
  （当前已登记的伪文件）。
- 拦截点：
  - `lookup`：根下识别 `search/index/where` -> 返魔法目录 fileid；
    在魔法目录下 `lookup <query>` -> 登记 fileid->query/键 的伪文件映射
  - `getattr`：魔法 id -> 返合成 fattr3（NF3REG，占位 size，规避「先算查询才能返属性」）
  - `read`（魔法伪文件）：query->embed->search->拼文本返回
  - `write`/`create`（在 `index/<key>`）：embed->upsert（每次 write 即 upsert，初版）
  - `readdir`：根拼魔法目录；魔法目录返已登记伪文件
- **不重写** id_to_fh/fh_to_id/path_to_id/fsinfo（default 透明支持魔法高位段）。
- 查询路径 percent-encode（空格等 escape）。

## 验收手段
集成测试（tempdir + `:memory:` + DummyEmbedder）：
- **exact-match 验证管线**（DummyEmbedder 下同文本->同向量->cosine=1->top-1）；
  语义质量（`async` 召回 `async io`）留给 TODO-04 真实 embedder
- index 写入后 search 伪文件 read 命中（top-1 == 写入键）
- 普通路径回归 OK（real FS 行为不退化）
- 手动挂载：`echo "rust async io">/mnt/index/k1 && cat /mnt/search/async` 命中

## 状态
pending

## 证据
（完成后填）

## 卡点
- 查询路径编码（空格 escape）
- readdir 确定性（魔法目录条目须稳定排序）
- getattr 高频：魔法伪文件用占位 size，不在此算查询
- 写入语义：`echo>` = create+write+close，初版每次 write 即 upsert

## 依赖
TODO-02, TODO-03
