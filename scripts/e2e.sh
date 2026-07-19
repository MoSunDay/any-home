#!/usr/bin/env bash
# End-to-end smoke test: start anyhome, mount it via NFSv3, exercise the magic
# paths (/index and /search), then assert and clean up.
#
# Requires an NFS-capable host: `mount.nfs` + the kernel nfs client. In
# sandboxes without them this script skips (exit 0) so CI stays green.
#
# Usage: scripts/e2e.sh [PORT]
set -euo pipefail

PORT="${1:-11111}"
HOST="127.0.0.1"
EXPORT="/"
MNT="${ANYHOME_MNT:-/mnt/anyhome}"
WORK="$(mktemp -d -t anyhome-e2e-XXXXXX)"

# Hard mount constraint: nfsserve multiplexes NFS+MOUNT+portmap on ONE port and
# runs no rpcbind/NLM, so we must pin vers/tcp/nolock/noacl + port=mountport.
MOUNT_OPTS="vers=3,tcp,nolock,noacl,port=${PORT},mountport=${PORT}"

cleanup() {
    if mountpoint -q "$MNT" 2>/dev/null; then umount "$MNT" || true; fi
    if [ -n "${SERVER_PID:-}" ]; then kill "$SERVER_PID" 2>/dev/null || true; fi
    rm -rf "$WORK"
}
trap cleanup EXIT

echo "==> building anyhome (debug)"
cargo build --workspace
TARGET_DIR="$(cargo metadata --format-version 1 --no-deps 2>/dev/null | python3 -c "import json,sys;print(json.load(sys.stdin)['target_directory'])")"
BIN="$TARGET_DIR/debug/anyhome"

echo "==> starting anyhome on ${HOST}:${PORT} (fs=$WORK/fs db=$WORK/mem.db)"
mkdir -p "$WORK/fs"
"$BIN" --fs-root "$WORK/fs" --db-path "$WORK/mem.db" --addr "${HOST}:${PORT}" --export "$EXPORT" \
    > "$WORK/server.log" 2>&1 &
SERVER_PID=$!

# Wait for the port to accept connections.
if ! timeout 20 bash -c "until (echo > /dev/tcp/${HOST}/${PORT}) 2>/dev/null; do sleep 0.2; done"; then
    echo "server did not come up; log:"; cat "$WORK/server.log"; exit 1
fi

if ! command -v mount.nfs >/dev/null 2>&1; then
    echo "==> mount.nfs not found / no NFS client — skipping live mount (exit 0)"
    exit 0
fi

echo "==> mounting ${HOST}:${EXPORT} at $MNT"
mkdir -p "$MNT"
mount.nfs -o "$MOUNT_OPTS" "${HOST}:${EXPORT}" "$MNT"

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

echo "==> ls /"
ls -la "$MNT"

echo "PASS: index -> search round-trip works over NFSv3"
