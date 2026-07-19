# TODO-03 DB schema + MemoryDb + Embedder 抽象

## 目标
用 libSQL 建 DB 层：schema 初始化、文本向量化入库、KNN 检索；
定义 `Embedder` trait + `DummyEmbedder`，让检索管线在无真实模型时可验证。

## 核心实现

### 第一步（硬门禁，应对零 CI 保护）—— vector 冒烟测试
core libsql 的 vector 测试**全是 `#[ignore]`**（`integration_tests.rs:764` vector_fuzz_test），
默认 `cargo test` 不跑任何 vector SQL。故 TODO-03 第一步**必须**写非-ignore 冒烟作为门禁：
```rust
#[tokio::test]
async fn smoke_vector_top_k() {
    let db = libsql::Database::open(":memory:").unwrap();
    let conn = db.connect().unwrap();
    conn.execute("CREATE TABLE items (id INTEGER PRIMARY KEY, emb FLOAT32(3))", ()).await.unwrap();
    conn.execute("CREATE INDEX items_emb_idx ON items ( libsql_vector_idx(emb) )", ()).await.unwrap();
    conn.execute("INSERT INTO items VALUES (?, vector(?))", libsql::params![1i64, "[1.0,1.0,1.0]"]).await.unwrap();
    conn.execute("INSERT INTO items VALUES (?, vector(?))", libsql::params![2i64, "[1.0,1.0,1.1]"]).await.unwrap();
    conn.execute("INSERT INTO items VALUES (?, vector(?))", libsql::params![3i64, "[9.0,9.0,9.0]"]).await.unwrap();
    let mut rows = conn.query("SELECT id FROM vector_top_k('items_emb_idx', vector(?), 2)",
                              libsql::params!["[1.0,1.0,1.0]"]).await.unwrap();
    assert_eq!(rows.next().await.unwrap().unwrap().get::<i64>(0).unwrap(), 1);
    assert_eq!(rows.next().await.unwrap().unwrap().get::<i64>(0).unwrap(), 2);
    assert!(rows.next().await.is_none());
}
```
**此测试不过 -> TODO-03 停摆，回 build.rs 核对 C 代码版本与 flags。**

### MemoryDb
- `MemoryDb{conn: Connection, dim: usize}`，derive Clone。**不包 Mutex**。
- open：`Builder::new_local(path).build().await? -> db.connect()?`（现代路径，非 deprecated）。
- 共享：`conn.clone()`（Arc 计数+1，底层同一 sqlite 连接，SQLITE_THREADSAFE=1 SERIALIZED 安全）。
- **不用显式 transaction**（`transaction()` 在共享连接上并发会冲突）；upsert/search 用单条 execute/query。

### schema（幂等）
- `dim` 字面量拼接进 SQL（**不能 bind**）：
  `CREATE TABLE IF NOT EXISTS memories(id INTEGER PRIMARY KEY, key TEXT UNIQUE, text TEXT, emb FLOAT32({dim}))`
- `CREATE INDEX IF NOT EXISTS memories_vec_idx ON memories ( libsql_vector_idx(emb) )`（**无 USING**，默认 metric=cosine）

### upsert
`INSERT INTO memories(key,text,emb) VALUES(?,?,vector(?)) ON CONFLICT(key) DO UPDATE SET text=excluded.text, emb=excluded.emb`
（第三参 = JSON 字符串 `"[f1,f2,...]"`）

### search
`SELECT id,text FROM vector_top_k('memories_vec_idx', vector(?), ?)`（第一参=索引名；结果无 distance 列，按距离升序）

### Embedder
- `db/embedder.rs`：`trait Embedder{async fn embed(&self,text:&str)->Result<Vec<f32>>; fn dim(&self)->usize;}`
- `DummyEmbedder`：确定性哈希到 dim 维，序列化为 `"[f1,f2,...]"`。

## 验收手段
单测（`:memory:` + DummyEmbedder）：
- schema 存在（重复 open 不报错）
- 插 3 条后 search 返 3 行无 panic
- **exact-match**：查询向量==某条时该条第一
- upsert 覆盖（同 key 二次写后 search 反映新值）
- **smoke_vector_top_k 通过**（硬门禁）

## 状态
pending

## 证据
（测试名 + passed；冒烟结果）

## 卡点
- vector 冒烟是第一道关，过不了则 TODO-03 全停。
- `libsql::Connection` 已确认 Send+Sync+Clone（`connection.rs:131-135`），不需 Arc/Mutex。

## 依赖
TODO-01
