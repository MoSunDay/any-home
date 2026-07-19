#!/usr/bin/env bash
# Reproducibly set up link_repos/* as read-only embedded dependencies.
#
# Each repo is cloned from upstream, then checked out to a pinned "memory
# baseline" commit (= upstream base + a repo-local memory commit on top).
#
#   libsql   : base 6f451a1f.. -> tip faf79716.. (memory baseline)
#   nfsserve : base c00c3614.. -> tip abe65c59.. (memory baseline)
#
# link_repos is gitignored at the anyhome root; this script rebuilds it from
# public upstreams so the build is reproducible without vendoring the deps.
set -euo pipefail

HERE="$(cd "$(dirname "$0")/.." && pwd)"
LINK="$HERE/link_repos"

LIBSQL_URL="${LIBSQL_URL:-https://github.com/tursodatabase/libsql.git}"
LIBSQL_TIP="faf79716c858bac3bd7cf694eaec3cec26523eac"

NFSSERVE_URL="${NFSSERVE_URL:-https://github.com/huggingface/nfsserve.git}"
NFSSERVE_TIP="abe65c594333a17464761243f876133acdd81956"

setup_repo() {
  local name="$1" url="$2" tip="$3"
  local dir="$LINK/$name"
  if [ -d "$dir/.git" ]; then
    echo "[bootstrap] $name: already present, fast-forwarding to $tip"
    git -C "$dir" fetch "$url" "$tip" >/dev/null 2>&1 || true
    git -C "$dir" checkout "$tip"
  else
    echo "[bootstrap] $name: cloning $url"
    git clone "$url" "$dir"
    git -C "$dir" checkout "$tip"
  fi
  echo "[bootstrap] $name: HEAD = $(git -C "$dir" rev-parse HEAD)"
}

setup_repo libsql   "$LIBSQL_URL"   "$LIBSQL_TIP"
setup_repo nfsserve "$NFSSERVE_URL" "$NFSSERVE_TIP"

echo "[bootstrap] done. verify with: cargo tree -p anyhome | grep -E 'tonic|hyper|rustls'"
