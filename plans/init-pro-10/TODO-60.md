# TODO-60 · 本机部署 + live e2e（运维步骤，无代码 commit）

**状态**：done

## 步骤与证据
1. `cargo build --release` → `target/release/anyhome`（6.4 MB）。
2. 常驻实例（dummy embedder，离线可复现）：
   `./target/release/anyhome --fs-root ./anyhome-fs --db-path ./anyhome.db
   --addr 127.0.0.1:11111 > anyhome-server.log 2>&1 &`（setsid 脱离会话）。
   `ss -ltnp`：127.0.0.1:11111 LISTEN。
3. `apt-get install -y nfs-common` 成功（mount.nfs 就位）；内核模块存在
   （/lib/modules/$(uname -r)/kernel/fs/nfs/nfsv3.ko）。
4. `scripts/e2e.sh --require-mount 21111`（独立端口避开常驻实例）exit 0：
   `PASS: index/search/where + where-prefix + agent-where + persistence + auth
   live all green over NFSv3`——**真实 mount，非 skip 路径**。
5. 常驻实例手工 mount 冒烟：`echo "..." > /mnt/anyhome/index/k` →
   `cat /mnt/anyhome/search/k` 往返成功（`k<TAB>deployed smoke test ...`）。
6. 部署实例**保持运行**；产物被 TODO-56 gitignore 条目覆盖（git status 干净）。

## 已知现象（非本轮引入）
虚拟查询文件读取尾部带填充空白（e2e 与冒烟输出中均可见）——anyhome 既有行为，
grep 型断言与本轮测试不受影响。
