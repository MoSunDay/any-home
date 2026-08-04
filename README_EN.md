[English](README_EN.md) | [中文](README.md)

# anyhome

**A mountable NFSv3 memory service for AI agents: turn "memory" into a local file system.**

## Why

anyhome turns "memory" into a local NFS service. AI agents can read, write, and search memory using the most basic file operations (`ls` / `cat` / `echo` / `vim` …) — no specialized SDK, no API at all.

NFS is natively supported by virtually every operating system and can be mounted anywhere. By exposing memory as an NFSv3 service on `localhost`, agents access memory purely through "read file / write file" — the most universal capability available. Shell, Python, Node, and GUI editors all work with zero adaptation.

## Design

Memory is split into two layers based on "whether it can be enumerated and located by path," then connected by "magic paths":

### 1. File System Layer — enumerable, path-locatable data

Any memory that is naturally tree-shaped, enumerable, and locatable by path exists directly as files/directories in the NFS view. Agents can `ls` to browse, `cat` to read, `echo` to write, and `mkdir` to organize — just like managing a real directory.

> e.g. `/agents/<id>/profile.md`, `/sessions/<id>/messages.log`, `/knowledge/<topic>/note.md`.

### 2. DB Layer — data that's hard to enumerate

Anything that cannot be enumerated and requires semantic / vector / full-text retrieval goes into a database (built on embedded libSQL — a SQLite fork with native vector retrieval support). This is "massive, non-enumerable" content not suitable for flat files.

### 3. Magic Paths — use NFS paths as queries

DB retrieval capabilities are disguised as file-system paths, so agents still only need file operations to query the DB:

- **Vector search**: `cat /search/<query>` — vector KNN search, the path is the query, file content is results (one line per result: `key<TAB>text`).
- **Write to index**: `echo "text" > /index/<key>` — writing embeds and upserts into the vector store.
- **Full-text search**: `cat /where/<word>` — FTS5 full-text MATCH search.
- **Key prefix filter**: `cat /where/key:<prefix>` — filter by key prefix.

### Multi-Agent Namespace Isolation

Each agent gets independent magic-path namespaces:
- `/agents/<id>/search/<q>` — KNN scoped to agent `<id>`
- `/agents/<id>/index/<key>` — writes belong to agent `<id>`
- `/agents/<id>/where/<cond>` — FTS/prefix scoped to agent `<id>`
- First write implicitly creates the agent.

Global `/search`, `/index`, `/where` map to the default agent (`agent_id = ""`), fully backward-compatible.

### Agent Identity Authentication (soft auth)

Optional. Register agent credentials binding `agent_id` to the caller's NFS AUTH_SYS `uid`. When enabled, callers accessing `/agents/<id>/...` must prove identity — a matching `uid` is required; impersonation returns `NFS3ERR_ACCES`; unauthorized agents are hidden from `/agents` listings. The default agent stays public. When no credentials are configured, auth is disabled (Open policy), fully backward-compatible.

## Architecture

```
                  +------------- agent (shell / editor / script) -------------+
                  |        ls / cat / echo  --  any file op                    |
                  +------------------------------+----------------------------+
                                                 |  mount localhost:NFSv3
                  +------------------------------v----------------------------+
                  |  anyhome NFSFileSystem impl (dispatcher)                   |
                  |   - normal path  -> filesystem layer (enumerable memory)   |
                  |   - magic path   -> query parse -> DB layer (vector/full)  |
                  +----------+--------------------------------+---------------+
                             |                                |
              +--------------v-----------+    +---------------v---------------+
              | link_repos/nfsserve      |    | link_repos/libsql             |
              | user-mode NFSv3 engine   |    | SQLite fork, native vectors   |
              | NFSFileSystem = boundary |    | vector() / libsql_vector_idx  |
              +--------------------------+    +-------------------------------+
```

## Quick Start

### 0. Prepare read-only embedded dependencies

`link_repos/{nfsserve,libsql}` are pinned, read-only embedded dependencies, already gitignored. Bootstrap them reproducibly:

```bash
scripts/bootstrap.sh
```

### 1. Build

```bash
cargo build          # default: dummy embedder, zero network dependencies
cargo build --features embed-fastembed   # real semantic embeddings (ONNX all-MiniLM-L6-v2, dim 384)
```

### 2. Start the service

```bash
cargo run -- --fs-root ./anyhome-fs --db-path ./anyhome.db --addr 127.0.0.1:11111
# with semantic retrieval:
cargo run --features embed-fastembed -- --embedder fastembed --db-path ./anyhome.db
# with agent auth:
cargo run -- --agent-uid botA:1001 --agent-uid botB:1002 --db-path ./anyhome.db
# or load from a credentials file:
cargo run -- --agent-creds /etc/anyhome/agents.conf
```

### 3. Mount (requires `mount.nfs` and kernel NFS client)

```bash
mkdir -p /mnt/anyhome
mount -t nfs -o port=11111,mountport=11111,nfsvers=3,nolock 127.0.0.1:/ /mnt/anyhome
```

### 4. Read/write memory with plain file operations

```bash
# write = embed + upsert into vector store
echo "hello anyhome" > /mnt/anyhome/index/greeting

# path is the query: read returns vector KNN results (one line per result: key<TAB>text)
cat /mnt/anyhome/search/hello
# => greeting	hello anyhome

# FTS5 full-text search
cat /mnt/anyhome/where/hello
# => greeting	hello anyhome

# key prefix filter
cat /mnt/anyhome/where/key:gre

# filesystem layer works as usual
echo "# my notes" > /mnt/anyhome/notes/readme.md
```

### 5. End-to-end smoke test

```bash
scripts/e2e.sh                          # skips if no mount.nfs
scripts/e2e.sh --require-mount          # hard-fails if no mount.nfs
ANYHOME_EMBEDDER=fastembed scripts/e2e.sh  # with semantic embeddings
```

### 6. Run tests

```bash
cargo test                              # 82 tests (default, zero dependencies)
cargo test --features embed-fastembed   # 86 tests
cargo clippy --all-targets -- -D warnings
```

## CLI Reference

```
anyhome [OPTIONS]

OPTIONS:
    --fs-root <PATH>           on-disk directory backing the filesystem layer (default ./anyhome-fs)
    --db-path <PATH>           libSQL database path; ":memory:" for ephemeral DB (default :memory:)
    --addr <IP:PORT>           listen address, single-port multiplex (default 127.0.0.1:11111)
    --export <PATH>            export path surfaced to NFS clients (default /)
    --embedder <KIND>          dummy (default) or fastembed (requires --features embed-fastembed)
    --dim <N>                  vector dimension for the dummy embedder (default 64; fastembed pins to 384)
    --agent-uid <ID:UID>       bind agent_id to caller AUTH_SYS uid (repeatable); enables auth
    --agent-creds <FILE>       credentials file, one id:uid per line, supports # comments; enables auth
```

Credentials file format (`--agent-creds`):

```
# agent credentials
botA:1001
botB:1002
```

## Embedding Backends

| Backend | Feature | Model | `/search` behavior | Network |
|---------|---------|-------|--------------------|---------|
| `dummy` (default) | — | deterministic hash | exact match only | none |
| `fastembed` | `embed-fastembed` | all-MiniLM-L6-v2 (dim 384, local ONNX) | real semantic KNN | model fetched once (or prefetch via `scripts/fetch-model.sh`) |

## Status

The core pipeline is built and fully tested:

- `cargo test` = **82 tests pass** (default, zero dependencies)
- `cargo test --features embed-fastembed` = **86 tests pass**
- `cargo clippy --all-targets -- -D warnings` = **0 warnings** (both feature combinations)

Feature checklist:

- **Read-only embedded dependencies**: `link_repos/{nfsserve,libsql}` pinned, reproducibly built via `scripts/bootstrap.sh`
- **NFS backend dispatch**: `AnyHomeFs` routes magic paths to the DB layer, everything else to the filesystem layer (`RealFs`)
- **Magic paths**: `/search/<q>` (vector KNN), `/index/<key>` (write = embed + upsert), `/where/<word>` (FTS5 MATCH) / `/where/key:<prefix>` (key prefix filter). `/search` and `/where` are read-only query views (write / setattr returns `ROFS`)
- **Multi-agent namespace isolation**: `/agents/<id>/{search,index,where}`, each with independent DB key space and search scope
- **Agent identity auth (soft auth)**: optional `--agent-uid` / `--agent-creds`; AUTH_SYS uid binding
- **Vector search**: libsql native `FLOAT32(dim)` + `libsql_vector_idx`, cosine distance KNN
- **Persistence**: memories written to `--db-path` survive restarts; embedding dimension stored in `meta` table, validated on startup; FTS5 index auto-rebuilt on startup if needed
- **CLI**: single-port service, fully configurable

## Known Limitations

- **`/index` write buffering**: write data is buffered in memory and upserted only on close; a crash may lose unflushed buffers (NFS clients typically re-`lookup` after reconnect, making this acceptable semantics).
- **Soft auth only**: NFSv3 AUTH_SYS `uid`/`gid` is client-asserted and **trivially spoofable** — this mechanism only guards against accidental misuse and cross-contamination, **not adversarial security**. `getattr`/`setattr` (pure metadata) are not authenticated. For strong auth, deploy behind a TLS/mTLS-terminating network front, or upgrade to RPCSEC_GSS/Kerberos.
- **root (uid 0) cannot safely use agent auth**: under AUTH_SYS, `uid 0` simultaneously means "anonymous caller" (AUTH_NULL) and "real root" — the two are indistinguishable. Therefore `--agent-uid` / `--agent-creds` **reject** binding `uid 0` — if allowed, all anonymous callers would gain that agent's access. Agent processes must run as a non-root uid.

## Project Structure

```
anyhome/
├── crates/anyhome/
│   ├── src/
│   │   ├── lib.rs          # library crate root
│   │   ├── main.rs         # binary entry point, builds AnyHomeFs
│   │   ├── cli.rs          # clap CLI definition
│   │   ├── dispatcher.rs   # AnyHomeFs: NFSFileSystem dispatch (real fs + magic paths)
│   │   ├── auth.rs         # AuthPolicy, CallerCred, AgentCred (soft auth)
│   │   ├── magic.rs        # magic-path encoding, fileid categorization, MagicRegistry
│   │   ├── backend/        # RealFs: on-disk filesystem layer
│   │   └── db/             # MemoryDb (libsql), embedder trait and implementations
│   └── tests/              # integration tests (10 files)
├── link_repos/             # read-only embedded dependencies (gitignored)
│   ├── nfsserve/           # user-mode NFSv3 engine (with anyhome fork patch)
│   └── libsql/             # SQLite fork with native vector retrieval
├── scripts/
│   ├── bootstrap.sh        # reproducible dependency setup + patch application
│   ├── e2e.sh              # end-to-end NFS mount smoke test
│   ├── fetch-model.sh      # prefetch fastembed model for offline use
│   └── patches/            # local fork patches applied on pinned upstream
├── features/changelog/     # iteration changelogs (design history)
├── Cargo.toml              # workspace root
└── rust-toolchain.toml     # MSRV pin (1.85.0)
```

Implementation notes:

- `link_repos/*` are read-only embedded dependencies (independent git repos, gitignored), **not** workspace members. nfsserve carries a small local fork patch (`scripts/patches/nfsserve-auth-identity.patch`), applied by `scripts/bootstrap.sh`, to expose AUTH_SYS caller identity to the `NFSFileSystem` trait.
- The `NFSFileSystem` trait of nfsserve is the sole integration boundary.

## Dependencies & Build Requirements

- Rust 1.85.0 (pinned via `rust-toolchain.toml`)
- Linux with NFS client support (`mount.nfs`), for mounting
- git + network access, for the first run of `scripts/bootstrap.sh`
