# TODO-07 e2e 脚本 + 使用文档

## 目标
脚本跑通全系统并断言，证明可上线；README 补「怎么用」。
这是计划完成的全局验收 gate。

## 核心实现
- `scripts/e2e.sh`（用 `scripts/` **不带点**——`.gitignore:27` 忽略 `.scripts`，不忽略 `scripts`）：
  1. 前置检查：`command -v mount.nfs || { apt install -y nfs-common; }`（当前环境 mount.nfs/rpcbind 全缺）
  2. 起 anyhome（端口 11111，导出 `/`，tempdir 作 fs_root，`:memory:` 或 tempdir db）
  3. mkdir /mnt/anyhome && **mount**（命令见下，硬约束）
  4. 三断言：
     - 真实层：`echo hi > /mnt/anyhome/real.txt && cat /mnt/anyhome/real.txt` == hi
     - `/index` 写：`echo "rust async io" > /mnt/anyhome/index/k1`
     - `/search` 命中：`cat /mnt/anyhome/search/async` 含 k1
  5. umount + kill + 清理 tempdir
- **mount 命令（不可改，nfsserve 无真 rpcbind）**：
  ```bash
  mount.nfs -o vers=3,tcp,nolock,noacl,port=11111,mountport=11111 localhost:/ /mnt/anyhome
  ```
  - `port=`/`mountport=` 缺一不可且相等（NFS/MOUNT/portmap 三协议 multiplex 同端口，`rpcwire.rs:48-53`）
  - `nolock` 必加（nfsserve 无 NLM 锁管理器）
  - `vers=3,tcp` 必加（nfsserve 只实现 NFSv3 over TCP）
- README「使用」节：前置（nfs-common）、启动命令、mount 命令、魔法路径约定、Mac 差异
  （`mount_nfs` + `nolocks` 复数，同样需 port=/mountport=）。

## 验收手段
- Linux 手动 `scripts/e2e.sh` 退出 0、三断言过
- README 链接有效

## 状态
pending

## 证据
（e2e 退出码、断言尾段）

## 卡点
- 需 root `mount.nfs`（CI 无权限 -> `e2e:manual`）
- 当前环境缺 mount.nfs/nfs-common，脚本须自装或前置检查
- Mac `mount_nfs` 语法不同，脚本先只 Linux

## 依赖
TODO-06
