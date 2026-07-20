# TODO-10 真实语义 embedder（fastembed/ONNX）

## 目标
用本地 ONNX 模型替换 DummyEmbedder，实现真正语义相似度（同义/近义召回），
解决 init-pro TODO-04 的 blocked-decision。

## 核心实现
- `embed-fastembed` feature（默认 off）+ optional `fastembed={version="4",optional=true}` dep。
- `db/fastembed.rs`：`FastEmbedder`（`FASTEMBED_DIM=384`，`all-MiniLM-L6-v2`，
  `spawn_blocking` 跑推理避免阻塞 async runtime）。
- `cli.rs`：`EmbedderKind` enum + `--embedder` flag。
- `main.rs`：`build_embedder()` 按枚举 + feature 构造（fastembed 不可用时回退 DummyEmbedder）。

## 模型
- `Qdrant/all-MiniLM-L6-v2-onnx`（HuggingFace）。
- sha `5f1b8cd78bc4fb444dd171e59b18f3a3af89a079`，5 文件，`model.onnx` ~90MB。
- `scripts/fetch-model.sh` 预取到 HF cache；测试自动检测缓存 + `HF_HUB_OFFLINE=1`。

## MSRV
- fastembed 传递依赖（image 0.25.10, idna_adapter 1.2.2, icu_*, built 0.8.1）要求 1.86+。
- Cargo.lock 钉 image@0.25.5, idna_adapter@1.1.0, idna@1.0.3, built@0.7.7 回到 1.85。

## 验收手段
`tests/fastembed_semantic.rs`（feature-gated，模型不可用时自跳过）：
- dim = 384
- 确定性（同输入同输出）
- 语义不等式：cos(hello,hi)=0.807 > cos(hello,greetings)=0.700 > cos(hello,banana)=0.288

## 状态
done

## 证据
- fastembed_semantic 3/3 passed。
- clippy clean（both feature configs）。

## 卡点
无。HuggingFace 主站不可达时用 hf-mirror.com 预取 + `HF_HUB_OFFLINE=1`。

## 依赖
TODO-08。
