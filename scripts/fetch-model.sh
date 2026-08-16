#!/usr/bin/env bash
# Prefetch the ONNX embedding model used by anyhome's `embed-fastembed`
# feature into the standard HuggingFace cache layout, so the model loads fully
# offline (no network, no API key).
#
# The fastembed crate normally pulls the model through `hf-hub`. That works on
# huggingface.co but in sandboxes where that host is unreachable, the usual
# fallback hf-mirror.com breaks hf-hub's resumable download
# ("Header Content-Range is missing"). This script sidesteps hf-hub entirely:
# it curls the 5 snapshot files straight into the HF cache dir and writes the
# refs/main pointer. The test harness (crates/anyhome/tests/fastembed_semantic.rs)
# then auto-detects this layout under HF_HOME + HF_HUB_OFFLINE=1.
#
# Model : Qdrant/all-MiniLM-L6-v2-onnx  (384-dim, model.onnx ~90MB, 5 files)
# Cache : $HOME/.cache/huggingface/hub/models--Qdrant--all-MiniLM-L6-v2-onnx
#
# Usage:
#   scripts/fetch-model.sh             # huggingface.co, or $HF_ENDPOINT if set
#   scripts/fetch-model.sh --mirror    # force https://hf-mirror.com
set -euo pipefail

REPO="Qdrant/all-MiniLM-L6-v2-onnx"
SHA="5f1b8cd78bc4fb444dd171e59b18f3a3af89a079"
FILES=(config.json model.onnx special_tokens_map.json tokenizer_config.json vocab.txt)

# Endpoint: --mirror forces hf-mirror.com; otherwise honour an explicit
# HF_ENDPOINT and fall back to huggingface.co.
ENDPOINT="https://huggingface.co"
if [ -n "${HF_ENDPOINT:-}" ]; then
    ENDPOINT="$HF_ENDPOINT"
fi
if [ "${1:-}" = "--mirror" ]; then
    ENDPOINT="https://hf-mirror.com"
fi

CACHE_ROOT="${HOME}/.cache/huggingface/hub"
MODEL_DIR="${CACHE_ROOT}/models--Qdrant--all-MiniLM-L6-v2-onnx"
SNAPSHOT_DIR="${MODEL_DIR}/snapshots/${SHA}"

# Skip if the big file already looks complete (>1 MiB); the other 4 are tiny.
if [ -f "$SNAPSHOT_DIR/model.onnx" ] \
   && [ "$(wc -c < "$SNAPSHOT_DIR/model.onnx")" -gt 1048576 ]; then
    echo "==> model already cached at $SNAPSHOT_DIR/model.onnx -- nothing to do"
    echo "==> use offline with:"
    echo "    export HF_HOME=$HOME/.cache/huggingface/hub"
    echo "    export HF_HUB_OFFLINE=1"
    exit 0
fi

echo "==> fetching $REPO @ $SHA via $ENDPOINT"
mkdir -p "$SNAPSHOT_DIR" "$MODEL_DIR/refs"

for f in "${FILES[@]}"; do
    url="${ENDPOINT}/${REPO}/resolve/${SHA}/${f}"
    echo "  -> $f"
    curl -fsSL --retry 3 -o "$SNAPSHOT_DIR/$f" "$url"
done

# Record the resolved commit so hf-hub treats this snapshot as current.
# hf-hub reads refs/main UNTRIMMED, so no trailing newline here (printf, not
# echo): a newline would make the prefetched cache invisible to fastembed.
printf '%s' "$SHA" > "$MODEL_DIR/refs/main"

echo "==> done. Use offline with:"
echo "    export HF_HOME=$HOME/.cache/huggingface/hub"
echo "    export HF_HUB_OFFLINE=1"
