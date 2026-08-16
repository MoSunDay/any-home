# TODO-69 · 部署回归：旧二进制数据安全 → 受控重启 → 新二进制全量矩阵

原则：**先在旧二进制上证明回归脚本不伤数据，再重启到新二进制**；部署记忆
（TODO-67 基准）全程字节不变；所有临时键走 `regress-12-*` 命名空间并三重
残留检查。脚本：`tmp/regress69_phaseA.sh` / `tmp/regress69_restart.sh` /
`tmp/regress69_phaseB.sh`。

## Phase A（旧二进制 pid 215315，端口 11111）—— PASS

只断言数据安全，不断言修复语义（旧二进制还没有）：

- A1 部署记忆 vs `tmp/audit67_dump.json` 字节全等。
- A2/A3 `regress-12-*` 写入回读 + `/where` 前缀枚举。
- A4 rm 后 DB + FTS 双残留为 0。
- A5 真实文件在 `anyhome-fs/` 的生灭。
- A6 全部回归操作后部署记忆仍字节不变。
- A7 清理后 DB 中 `regress-12-%` 计数与 FTS 残留均为 0。

脚本卫生修复：`ls | grep -q` 在 pipefail 下会 SIGPIPE（rc=141）→ 改
`grep >/dev/null` 读全量输入。

## 受控重启（R1..R4）

`cargo build --release` → 停旧 → 同 flags 起新。踩坑实录：重启脚本
`pgrep -f 'target/release/anyhome'` 同时命中了 setsid 包装 shell（其命令行
也含二进制路径），`head -1` 杀的是包装进程，真服务器仍在监听 → 新进程
`Address already in use` 而脚本误报 OK。修复：按 `/proc/<pid>/exe` 精确匹配
解析服务器 pid。人工接管后：kill 215315 → 端口释放 → 新二进制 pid 701435
以逐字相同的 flags 启动，日志确认 Dummy embedder / dim 64 / Open policy。

## Phase B（新二进制 pid 701435）—— PASS

- B1 部署记忆跨重启字节全等（写穿 DB 的活体证据）。
- B2/B3 真实 attr size 上线（stat == read == expected；`/where` 不再 1MiB）。
- B4 覆写无 stale tail；B5 O_TRUNC 落 DB（text = ''）。
- B6 rm 三重检查（枚举 / `/where` / DB+FTS）。
- B7 `/search_k2` 上限 + UTF-8 查询（行数按 `grep -vc '^$'` 计——魔法文件
  行间空行分隔）。
- B8 真实文件生命周期；B9 部署记忆全程不变；B10 清理后 DB+FTS+FS 三重零残留。

## 缺陷 C 线上复验（新二进制）

`mkdir agents/search` → EEXIST；`mkdir -p agents` → rc 0（GNU 按语义把 EXIST
当成功并回退 lookup）；根 readdir 虚拟四项齐全；`anyhome-fs/` 不生长任何
真实目录；DB 无残留 agent 行。
