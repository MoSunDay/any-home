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
MNT2="${ANYHOME_MNT2:-/mnt/anyhome-auth}"
T=$'\t'   # literal tab for grep assertions on "<key>\t<text>" output
EMBEDDER="${ANYHOME_EMBEDDER:-dummy}"
WORK="$(mktemp -d -t anyhome-e2e-XXXXXX)"

# Hard mount constraint: nfsserve multiplexes NFS+MOUNT+portmap on ONE port and
# runs no rpcbind/NLM, so we must pin vers/tcp/nolock/noacl + port=mountport.
MOUNT_OPTS="vers=3,tcp,nolock,noacl,port=${PORT},mountport=${PORT}"

# HF cache for the fastembed ONNX model (Qdrant's mirror of all-MiniLM-L6-v2).
# Only consulted when EMBEDDER=fastembed; reused offline to avoid any download.
HF_CACHE_DIR="${HF_HOME:-$HOME/.cache/huggingface}/hub/models--Qdrant--all-MiniLM-L6-v2-onnx"

cleanup() {
    if mountpoint -q "$MNT2" 2>/dev/null; then umount -f "$MNT2" 2>/dev/null || true; fi
    if mountpoint -q "$MNT" 2>/dev/null; then umount -f "$MNT" 2>/dev/null || true; fi
    if [ -n "${AUTH_PID:-}" ]; then kill "$AUTH_PID" 2>/dev/null || true; fi
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

# ---- MOUNT export-name enforcement (negative case) --------------------------
echo "==> asserting MOUNT enforces the export name (wrong export must be refused)"
mkdir -p "$WORK/mnt-bad"
if mount.nfs -o "$MOUNT_OPTS" "${HOST}:/definitely-not-the-export" "$WORK/mnt-bad" 2>/dev/null; then
    echo "FAIL: server accepted a mount for a wrong export name"; exit 1
fi
echo "    -> wrong export refused as expected"

# ---- /index -> /search round-trip -------------------------------------------
echo "==> /index/greeting <- 'hello anyhome'"
echo "hello anyhome" > "$MNT/index/greeting"

# Query = the same text (percent-encoded space); exact-match ranks top-1 under
# the DummyEmbedder, so the first result line must be "greeting\thello anyhome".
QUERY="$MNT/search/hello%20anyhome"
echo "==> cat $QUERY"
cat "$QUERY"

echo "==> asserting top-1 is 'greeting<TAB>hello anyhome'"
if ! grep -q "^greeting${T}hello anyhome$" "$QUERY"; then
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
if ! grep -q "^greeting${T}" "$WHERE_QUERY"; then
    echo "FAIL: expected key 'greeting' in /where results"; exit 1
fi
if ! grep -q "^second${T}" "$WHERE_QUERY"; then
    echo "FAIL: expected key 'second' in /where results"; exit 1
fi

# ---- /where/key:<prefix> prefix filtering -----------------------------------
echo "==> /where/key:gre (prefix filter — only keys starting with 'gre')"
PREFIX_QUERY="$MNT/where/key:gre"
cat "$PREFIX_QUERY"
echo "==> asserting /where/key:gre returns 'greeting' but excludes 'second'"
if ! grep -q "^greeting${T}" "$PREFIX_QUERY"; then
    echo "FAIL: expected 'greeting' in /where/key:gre results"; exit 1
fi
if grep -q "^second${T}" "$PREFIX_QUERY"; then
    echo "FAIL: 'second' must not appear in /where/key:gre (no 'gre' prefix)"; exit 1
fi

echo "==> ls /"
ls -la "$MNT"

# ---- /agents/<id>/index -> /agents/<id>/search round-trip -------------------
echo "==> /agents/botA/index/greeting <- 'agent A memory'"
mkdir -p "$MNT/agents/botA/index"
echo "agent A memory" > "$MNT/agents/botA/index/greeting"

echo "==> /agents/botB/index/greeting <- 'agent B memory'"
mkdir -p "$MNT/agents/botB/index"
echo "agent B memory" > "$MNT/agents/botB/index/greeting"

echo "==> ls /agents/"
ls "$MNT/agents/"

echo "==> asserting /agents/ lists botA and botB"
if ! ls "$MNT/agents/" | grep -q "botA"; then
    echo "FAIL: expected botA in /agents/ listing"; exit 1
fi
if ! ls "$MNT/agents/" | grep -q "botB"; then
    echo "FAIL: expected botB in /agents/ listing"; exit 1
fi

# Agent A search should only see agent A's content.
A_QUERY="$MNT/agents/botA/search/agent%20memory"
echo "==> cat $A_QUERY"
cat "$A_QUERY"
echo "==> asserting agent A search returns 'agent A memory'"
if ! grep -q "^greeting${T}agent A memory$" "$A_QUERY"; then
    echo "FAIL: expected 'greeting<TAB>agent A memory' in agent A search"; exit 1
fi

# Agent B search should only see agent B's content.
B_QUERY="$MNT/agents/botB/search/agent%20memory"
echo "==> cat $B_QUERY"
cat "$B_QUERY"
echo "==> asserting agent B search returns 'agent B memory'"
if ! grep -q "^greeting${T}agent B memory$" "$B_QUERY"; then
    echo "FAIL: expected 'greeting<TAB>agent B memory' in agent B search"; exit 1
fi

# ---- /agents/<id>/where agent-scoped FTS ------------------------------------
echo "==> /agents/botA/where/agent (agent-scoped FTS isolation)"
A_WHERE="$MNT/agents/botA/where/agent"
cat "$A_WHERE"
echo "==> asserting botA /where finds own content but not botB's"
if ! grep -q "^greeting${T}agent A memory" "$A_WHERE"; then
    echo "FAIL: expected 'agent A memory' in botA scoped /where"; exit 1
fi
if grep -q "agent B memory" "$A_WHERE"; then
    echo "FAIL: botA scoped /where leaked botB content"; exit 1
fi

# ---- restart persistence -----------------------------------------------------
echo "==> testing persistence: unmount -> restart server -> remount"
umount -f "$MNT" 2>/dev/null || true
kill "$SERVER_PID" 2>/dev/null
wait "$SERVER_PID" 2>/dev/null || true

echo "==> restarting anyhome with same db-path ($WORK/mem.db)"
"${SERVER_ENV[@]}" "$BIN" --fs-root "$WORK/fs" --db-path "$WORK/mem.db" \
    --addr "${HOST}:${PORT}" --export "$EXPORT" --embedder "$EMBEDDER" \
    > "$WORK/server.log" 2>&1 &
SERVER_PID=$!
if ! timeout 20 bash -c "until (echo > /dev/tcp/${HOST}/${PORT}) 2>/dev/null; do sleep 0.2; done"; then
    echo "server did not restart; log:"; cat "$WORK/server.log"; exit 1
fi

echo "==> remounting after restart"
mount.nfs -o "$MOUNT_OPTS" "${HOST}:${EXPORT}" "$MNT"

echo "==> asserting memories persisted across restart"
RESTART_WHERE="$MNT/where/hello"
cat "$RESTART_WHERE"
if ! grep -q "^greeting${T}" "$RESTART_WHERE"; then
    echo "FAIL: 'greeting' missing from /where after restart — persistence broken"; exit 1
fi
if ! grep -q "^second${T}" "$RESTART_WHERE"; then
    echo "FAIL: 'second' missing from /where after restart — persistence broken"; exit 1
fi

# ---- auth live test (soft AUTH_SYS authorization) ---------------------------
AUTH_PORT=$((PORT + 1))
echo "==> starting auth-enabled anyhome on ${HOST}:${AUTH_PORT} (--agent-uid botA:1001)"
mkdir -p "$WORK/fs-auth"
"${SERVER_ENV[@]}" "$BIN" --fs-root "$WORK/fs-auth" --db-path "$WORK/auth.db" \
    --addr "${HOST}:${AUTH_PORT}" --export "$EXPORT" --embedder "$EMBEDDER" \
    --agent-uid "botA:1001" \
    > "$WORK/auth-server.log" 2>&1 &
AUTH_PID=$!
if ! timeout 20 bash -c "until (echo > /dev/tcp/${HOST}/${AUTH_PORT}) 2>/dev/null; do sleep 0.2; done"; then
    echo "auth server did not come up; log:"; cat "$WORK/auth-server.log"; exit 1
fi

echo "==> mounting auth-enabled export at $MNT2"
mkdir -p "$MNT2"
MOUNT_OPTS2="vers=3,tcp,nolock,noacl,port=${AUTH_PORT},mountport=${AUTH_PORT}"
mount.nfs -o "$MOUNT_OPTS2" "${HOST}:${EXPORT}" "$MNT2"

# The mount process runs as root (uid 0). Since uid 0 cannot be bound to an
# agent (it denotes anonymous/AUTH_NULL), the caller is anonymous and confined
# to the public default agent — exactly what we verify below.
echo "==> asserting default agent stays public for anonymous caller"
echo "open-data" > "$MNT2/index/public"
AUTH_SEARCH="$MNT2/search/open-data"
cat "$AUTH_SEARCH"
if ! grep -q "^public${T}open-data" "$AUTH_SEARCH"; then
    echo "FAIL: default agent must stay public even under auth"; exit 1
fi

echo "==> asserting anonymous caller CANNOT reach /agents/botA/ (NFS3ERR_ACCES)"
if ls "$MNT2/agents/botA/" >/dev/null 2>&1; then
    echo "FAIL: anonymous caller reached /agents/botA/ — auth isolation broken"; exit 1
fi
echo "    -> access denied as expected"

echo "==> asserting /agents/ listing hides botA from unauthorized caller"
if ls "$MNT2/agents/" 2>/dev/null | grep -q "botA"; then
    echo "FAIL: /agents/ listing leaked botA to unauthorized caller"; exit 1
fi

echo "PASS: index/search/where + where-prefix + agent-where + persistence + auth + export-name enforcement live all green over NFSv3"
