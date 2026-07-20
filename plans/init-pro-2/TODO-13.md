# TODO-13 recall 评测脚手架

## 目标
建立可量化、可回归的语义召回评测，证明真实 embedder 显著优于 dummy baseline，
防止未来回归。

## 核心实现
- `tests/recall.rs`——10 对 (query, key+text) 改写集（query 与 key 的 text 是同义改写，
  非字面匹配）。
- `recall_at_k()` 计算 top-K 命中率。
- 两个测试：
  1. `dummy_embedder_baseline_fails_paraphrase_recall`（非 gated，断言 dummy recall@3 < 0.5，
     证明测试有区分力）
  2. `fastembed::fastembed_clears_recall_bar`（gated，断言 fastembed recall@3 >= 0.7）

## 验收手段
- dummy baseline recall@3 = 0.30 < 0.5 ✓（baseline 确实差）
- fastembed recall@3 = 1.00 >= 0.7 ✓（真实语义召回有效）

## 状态
done

## 证据
- `recall.rs` baseline 1/1 passed。
- gated fastembed recall 1/1 passed（recall@3=1.00）。

## 卡点
无。

## 依赖
TODO-10。
