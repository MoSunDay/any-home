# TODO-04 真实 Embedder（语义模型接入）

## 目标
为 `Embedder` trait 提供真实语义向量实现，使 `/search` 返回语义相关结果。
**非主线阻塞**：TODO-05/06 可先用 DummyEmbedder。

## 核心实现
方案二选一（**需决策**）：
- (A) 本地 `ort`(ONNX) 加载 all-MiniLM-L6-v2(dim 384)：离线低延迟，拉原生依赖 + 模型文件分发
- (B) `reqwest` 打 embeddings API：最简，需网络/key

实现 `LocalEmbedder` 或 `ApiEmbedder` 满足 `Embedder` trait。
**dim 必须与 TODO-03 schema 对齐**（改模型需迁移 DB）。

## 验收手段
单测：
- `embed("hello")`~`embed("hi")` 余弦 > `embed("hello")`~`embed("banana")`
- dim 与 schema 一致
- 与 TODO-03 联动 spot check 召回顺序

## 状态
**blocked-decision**

## 证据
（决策后填）

## 卡点
**需决策** (A) vs (B)——维度：是否允许联网 / 接受原生依赖 / 模型文件分发方式。
主线（TODO-05/06/07）用 DummyEmbedder，本 TODO 可延后到 TODO-06 之前。

## 依赖
TODO-03
