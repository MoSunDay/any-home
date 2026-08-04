#!/usr/bin/env bash
# Reproducibly set up link_repos/* as read-only embedded dependencies.
#
# Each repo is cloned from upstream (shallow) and checked out to a pinned
# commit. The pins below are the upstream HEADs at the time of init-pro-3; the
# anyhome code builds against these bases directly. (An earlier revision pinned
# "memory baseline" overlay commits that were never published; they turned out
# to be unnecessary for the build, so we now pin the plain upstream commits.)
#
#   libsql   : 6f451a1f.. (upstream HEAD)
#   nfsserve : c00c3614.. (upstream HEAD) + anyhome local patches (see below)
#
# Local patches: nfsserve is forked minimally to expose AUTH_SYS caller
# identity to NFSFileSystem implementations (init-pro-5, see
# features/changelog/2026-07-27/init-pro-5.md). The patch lives in
# scripts/patches/ and is applied on top of the pinned upstream commit so the
# build stays reproducible without publishing a fork.
#
# link_repos is gitignored at the anyhome root; this script rebuilds it from
# public upstreams so the build is reproducible without vendoring the deps.
set -euo pipefail

HERE="$(cd "$(dirname "$0")/.." && pwd)"
LINK="$HERE/link_repos"

LIBSQL_URL="${LIBSQL_URL:-https://github.com/tursodatabase/libsql.git}"
LIBSQL_TIP="6f451a1fabacbcbc9960b232b4c1605a5021979b"

NFSSERVE_URL="${NFSSERVE_URL:-https://github.com/huggingface/nfsserve.git}"
NFSSERVE_TIP="c00c3614184bc79010c0aa3b69c2cdde1c5fca77"

setup_repo() {
  local name="$1" url="$2" tip="$3"
  local dir="$LINK/$name"
  if [ -d "$dir/.git" ]; then
    echo "[bootstrap] $name: already present, fast-forwarding to $tip"
    git -C "$dir" fetch "$url" "$tip" >/dev/null 2>&1 || true
    git -C "$dir" checkout "$tip"
  else
    echo "[bootstrap] $name: cloning $url"
    git clone --depth 1 "$url" "$dir"
    git -C "$dir" checkout "$tip"
  fi
  echo "[bootstrap] $name: HEAD = $(git -C "$dir" rev-parse HEAD)"
}

setup_repo libsql   "$LIBSQL_URL"   "$LIBSQL_TIP"
setup_repo nfsserve "$NFSSERVE_URL" "$NFSSERVE_TIP"

# Apply anyhome's local fork patches on top of the pinned upstream commits.
# Idempotent via a marker file; hard-fails on upstream drift so silent breakage
# can never pass.
apply_patch() {
  local repo="$1" patch="$2" marker="$3"
  local dir="$LINK/$repo"
  if [ -e "$dir/$marker" ]; then
    echo "[bootstrap] $repo: $(basename "$patch") already applied (marker $marker present)"
    return 0
  fi
  if git -C "$dir" apply --check "$patch" 2>/dev/null; then
    git -C "$dir" apply "$patch"
    echo "[bootstrap] $repo: applied $(basename "$patch")"
  else
    echo "[bootstrap] ERROR: $repo patch $(basename "$patch") did not apply cleanly (upstream drift?)" >&2
    exit 1
  fi
}

# init-pro-5: expose AUTH_SYS caller identity (uid/gid/machinename/client_addr)
# to NFSFileSystem methods via a task-local in a new public `identity` module.
apply_patch nfsserve "$HERE/scripts/patches/nfsserve-auth-identity.patch" "src/identity.rs"

echo "[bootstrap] done. verify with: cargo tree -p anyhome | grep -E 'tonic|hyper|rustls'"
