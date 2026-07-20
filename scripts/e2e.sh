#!/usr/bin/env bash
# End-to-end smoke test: start anyhome, mount it via NFSv3, exercise the magic
# paths (/index, /search, /where), then assert and clean up.
#
# Requires an NFS-capable host: `mount.nfs` + the kernel nfs client. By default
# this script skips (exit 0) when `mount.nfs` is missing, so CI stays green on
# NFS-less sandboxes. Pass --require-mount (or set ANYHOME_REQUIRE_MOUNT=1) to
# turn a missing `mount.nfs` into a hard failure (exit 1), for environments
# that are expected to have a working NFS client.
#
# Embedder backend is chosen via ANYHOME_EMBEDDER (default: dummy). With
# fastembed the build enables the embed-fastembed feature and, if the model
# cache is present, the server runs fully offline against it.
#
# Usage: scripts/e2e.sh [--require-mount] [PORT]
set -euo pipefail

# ---- argument parsing -------------------------------------------------------
PORT=""
REQUIRE_MOUNT="${ANYHOME_REQUIRE_MOUNT:-0}"
while [ $# -gt 0 ]; do
    case "$1" in
        --require-mount)
            REQUIRE_MOUNT=1
            shift
            ;;
        --help|-h)
            echo "Usage: $0 [--require-mount] [PORT]"
            echo "  --require-mount   fail (exit 1) instead of skipping when mount.nfs is missing"
            echo "  PORT              listen port for the anyhome server (default 11111)"
            echo "Env: ANYHOME_REQUIRE_MOUNT=1, ANYHOME_EMBEDDER=<dummy|fastembed>, ANYHOME_MNT=..."
            exit 0
            ;;
        --*)
            echo "unknown flag: $1" >&2
            exit 2
            ;;
        *)
            if [ -z "$PORT" ]; then
                PORT="$1"
            else
                echo "unexpected extra positional arg: $1" >&2
                exit 2
            fi
            shift
            ;;
    esac
done
PORT="${PORT:-11111}"

# ---- configuration ----------------------------------------------------------
HOST="127.0.0.1"
EXPORT="/"
MNT="${ANYHOME_MNT:-/mnt/anyhome}"
EMBEDDER="${ANYHOME_EMBEDDER:-dummy}"
WORK="$(mktemp -d -t anyhome-e2e-XXXXXX)"

# Hard mount constraint: nfsserve multiplexes NFS+MOUNT+portmap on ONE port and
# runs no rpcbind/NLM, so we must pin vers/tcp/nolock/noacl + port=mountport.
MOUNT_OPTS="vers=3,tcp,nolock,noacl,port=${PORT},mountport=${PORT}"

# HF cache for the fastembed ONNX model (Qdrant's mirror of all-MiniLM-L6-v2).
# Only consulted when EMBEDDER=fastembed; reused offline to avoid any download.
HF_CACHE_DIR="${HF_HOME:-$HOME/.cache/huggingface}/hub/models--Qdrant--all-MiniLM-L6-v2-onnx"

cleanup() {
    if mountpoint -q "$MNT" 2>/dev/null; then umount "$MNT" || true; fi
    if [ -n "${SERVER_PID:-}" ]; then kill "$SERVER_PID" 2>/dev/null || true; fi
    rm -rf "$WORK"
}
trap cleanup EXIT

# ---- build ------------------------------------------------------------------
if [ "$EMBEDDER" = "fastembed" ]; then
    echo "==> building anyhome (debug, embed-fastembed) -- semantic embeddings active"
    cargo build --workspace --features embed-fastembed
else
    echo "==> building anyhome (debug, embedder=${EMBEDDER})"
    cargo build --workspace
fi
TARGET_DIR="$(cargo metadata --format-version 1 --no-deps 2>/dev/null | python3 -c "import json,sys;print(json.load(sys.stdin)['target_directory'])")"
BIN="$TARGET_DIR/debug/anyhome"

# ---- start server -----------------------------------------------------------
echo "==> starting anyhome on ${HOST}:${PORT} (fs=$WORK/fs db=$WORK/mem.db embedder=${EMBEDDER})"
mkdir -p "$WORK/fs"

# When using fastembed, reuse a prefetched model cache offline so the server
# never needs network. Only set HF_HOME/HF_HUB_OFFLINE when the cache actually
# exists; without it fastembed will try to download (and fail offline).
SERVER_ENV=()
if [ "$EMBEDDER" = "fastembed" ] && [ -d "$HF_CACHE_DIR" ]; then
    echo "    -> using cached ONNX model at $HF_CACHE_DIR (HF_HUB_OFFLINE=1)"
    SERVER_ENV+=(env "HF_HOME=${HF_HOME:-$HOME/.cache/huggingface}" HF_HUB_OFFLINE=1)
fi

"${SERVER_ENV[@]}" "$BIN" --fs-root "$WORK/fs" --db-path "$WORK/mem.db" \
    --addr "${HOST}:${PORT}" --export "$EXPORT" --embedder "$EMBEDDER" \
    > "$WORK/server.log" 2>&1 &
SERVER_PID=$!

# Wait for the port to accept connections.
if ! timeout 20 bash -c "until (echo > /dev/tcp/${HOST}/${PORT}) 2>/dev/null; do sleep 0.2; done"; then
    echo "server did not come up; log:"; cat "$WORK/server.log"; exit 1
fi

# ---- NFS client check -------------------------------------------------------
# Default: skip (exit 0) when the host has no NFS client, so CI stays green on
# sandboxes. With --require-mount, the absence of mount.nfs is a hard failure.
if ! command -v mount.nfs >/dev/null 2>&1; then
    if [ "$REQUIRE_MOUNT" = "1" ]; then
        echo "==> mount.nfs not found / no NFS client -- required but unavailable (--require-mount)"
        echo "FAIL: --require-mount is set but mount.nfs is missing"
        exit 1
    fi
    echo "==> mount.nfs not found / no NFS client -- skipping live mount (exit 0)"
    exit 0
fi

echo "==> mounting ${HOST}:${EXPORT} at $MNT"
mkdir -p "$MNT"
mount.nfs -o "$MOUNT_OPTS" "${HOST}:${EXPORT}" "$MNT"

# ---- /index -> /search round-trip -------------------------------------------
echo "==> /index/greeting <- 'hello anyhome'"
echo "hello anyhome" > "$MNT/index/greeting"

# Query = the same text (percent-encoded space); exact-match ranks top-1 under
# the DummyEmbedder, so the first result line must be "greeting\thello anyhome".
QUERY="$MNT/search/hello%20anyhome"
echo "==> cat $QUERY"
cat "$QUERY"

echo "==> asserting top-1 is 'greeting<TAB>hello anyhome'"
if ! grep -q "^greeting	hello anyhome$" "$QUERY"; then
    echo "FAIL: expected 'greeting<TAB>hello anyhome' in /search results"; exit 1
fi

# ---- /where: full-text lookup across all indexed memories -------------------
# Add a second memory that also contains "hello" so an FTS5 MATCH for "hello"
# must surface both keys regardless of result ordering.
echo "==> /index/second <- 'hello world'"
echo "hello world" > "$MNT/index/second"

WHERE_QUERY="$MNT/where/hello"
echo "==> cat $WHERE_QUERY"
cat "$WHERE_QUERY"

echo "==> asserting /where/hello mentions both keys (greeting, second)"
if ! grep -q "^greeting	" "$WHERE_QUERY"; then
    echo "FAIL: expected key 'greeting' in /where results"; exit 1
fi
if ! grep -q "^second	" "$WHERE_QUERY"; then
    echo "FAIL: expected key 'second' in /where results"; exit 1
fi

echo "==> ls /"
ls -la "$MNT"

echo "PASS: index -> search + where round-trip works over NFSv3"
