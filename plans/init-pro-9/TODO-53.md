# TODO-53 · unwrap→可恢复（Result 返回侧）：realfs 5 处 + db count 1 处

**状态**：done（commit a697012）

## 变更
- `src/backend/realfs.rs`：5 处 `self.state.lock().unwrap()` →
  `self.state.lock().map_err(|_| nfsstat3::NFS3ERR_IO)?`（`path_of`/
  `register_with_meta`/`remove` ×2/`rename`）。沿用文件内既有 28 处
  `NFS3ERR_IO` 映射惯例；`path_of` 处加注解释（守卫内容仅 id/path 簿记，
  HashMap 一致性之外无不变量，降级优于整 server 崩溃）。
- `src/db/schema.rs` `count()`：`rows.next()....unwrap()`（Option）→
  `.ok_or_else(|| anyhow!("count query returned no row for {table}"))?`。
  `SELECT count(*)` 恒返回一行，空行属真实异常，显式报错而非 panic。
- 新增锁毒恢复测试（realfs.rs 内联）：clone 共享 `Arc<Mutex<Maps>>`，线程持锁
  panic 染毒，断言 `lookup`/`getattr`/`create` 返回 `NFS3ERR_IO` 而非 panic。

## 验证
- `cargo test --workspace`：121 → **122 passed**；`--features embed-fastembed`：
  125 → **126 passed**；clippy 零警告。
