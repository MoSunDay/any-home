//! Metadata authorization integration tests (TODO-41/44, init-pro-7).
//!
//! Closes the size oracle found by the TODO-40 spike: `lookup` registers a
//! `MagicEntry` in the process-wide registry under an *authorized* caller,
//! and fileids are offline-derivable (FNV-1a of `"{agent}\0{key}"`), so an
//! *unauthenticated* caller holding/guessing the fileid could previously
//! `getattr`/`setattr` it and read the real stored byte count — contradicting
//! init-pro-5's "leaked handles are converged" semantics. These tests pin the
//! corrected auth matrix:
//!
//! | fileid kind                    | getattr / setattr under Enforced    |
//! |--------------------------------|-------------------------------------|
//! | CAT_INDEX file, resolvable     | gated by `require_agent` (ACCES)    |
//! | CAT_INDEX file, unresolvable   | zero-sized attrs, public (stale-handle shape preserved) |
//! | magic dirs / search / where    | virtual attrs, public (no per-agent data) |
//! | real FS                        | untouched (D4 narrowed to this)     |
//! | default agent (`/index`)       | always public                       |
//!
//! Under `Open` (no credentials) nothing changes — the init-pro-6 collapse
//! point. Under Enforced, read/write were already gated (init-pro-5); only
//! metadata needed closing.

use std::future::Future;
use std::sync::Arc;

use nfsserve::identity::{CallerIdentity, CALLER};
use nfsserve::nfs::{fileid3, filename3, nfsstat3, sattr3};
use nfsserve::vfs::NFSFileSystem;
use tempfile::tempdir;

use anyhome::auth::{AgentCred, AuthPolicy};
use anyhome::db::{DummyEmbedder, Embedder, MemoryDb};
use anyhome::{AnyHomeFs, RealFs};

fn nm(s: &[u8]) -> filename3 {
    s.into()
}

const ACCES: u32 = nfsstat3::NFS3ERR_ACCES as u32;

/// Build an `AnyHomeFs` backed by a temp dir + in-memory DB + dummy embedder.
async fn make() -> AnyHomeFs {
    let root = tempdir().expect("tempdir").keep();
    let real = RealFs::new(root).unwrap();
    let db = MemoryDb::open(":memory:", 16).await.unwrap();
    let emb: Arc<dyn Embedder> = Arc::new(DummyEmbedder::new(16));
    AnyHomeFs::new(real, db, emb)
}

fn enforced() -> AuthPolicy {
    AuthPolicy::from_creds(&[AgentCred::new("botA", 1001), AgentCred::new("botB", 1002)])
}

/// Run `fut` as if invoked by AUTH_SYS caller `uid`.
async fn as_caller<R>(uid: u32, fut: impl Future<Output = R>) -> R {
    let id = CallerIdentity {
        uid,
        gid: uid,
        machinename: format!("host{uid}").into_bytes(),
        client_addr: format!("127.0.0.1:{p}", p = 9000 + uid),
    };
    CALLER.scope(id, fut).await
}

/// Resolve `/agents/<agent>/<sub>/<name>` from root (caller must be the
/// owning agent; wrap in [`as_caller`]).
async fn agent_file(fs: &AnyHomeFs, agent: &str, sub: &str, name: &str) -> fileid3 {
    let root = fs.root_dir();
    let agents = fs.lookup(root, &nm(b"agents")).await.unwrap();
    let dir = fs.lookup(agents, &nm(agent.as_bytes())).await.unwrap();
    let sub_dir = fs.lookup(dir, &nm(sub.as_bytes())).await.unwrap();
    fs.lookup(sub_dir, &nm(name.as_bytes())).await.unwrap()
}

/// The oracle, pre-fix: an authorized lookup registers the entry, then an
/// unauthenticated `getattr` on the same fileid returned the real byte size.
#[tokio::test]
async fn size_oracle_closed_for_unauthenticated_getattr() {
    let fs = make().await.with_auth(enforced());
    let secret = b"classified memo of botA";

    // botA legitimately resolves and writes its index file (this is what
    // seeds the process-wide registry entry — the oracle's precondition).
    let id = as_caller(1001, agent_file(&fs, "botA", "index", "secret")).await;
    as_caller(1001, fs.write(id, 0, secret)).await.unwrap();

    // The owner still sees the real size.
    let attr = as_caller(1001, fs.getattr(id)).await.expect("owner getattr");
    assert_eq!(attr.size, secret.len() as u64);

    // botB (and an anonymous uid 0) probing the same handle -> ACCES, not size.
    let err = as_caller(1002, fs.getattr(id)).await.unwrap_err();
    assert_eq!(err as u32, ACCES, "leaked handle must not leak byte size");
    let err = as_caller(0, fs.getattr(id)).await.unwrap_err();
    assert_eq!(err as u32, ACCES, "anonymous callers are confined to the default agent");
}

/// setattr on an agent-scoped index file is gated identically.
#[tokio::test]
async fn setattr_gated_like_getattr() {
    let fs = make().await.with_auth(enforced());
    let id = as_caller(1001, agent_file(&fs, "botA", "index", "secret")).await;
    as_caller(1001, fs.write(id, 0, b"data")).await.unwrap();

    let err = as_caller(1002, fs.setattr(id, sattr3::default()))
        .await
        .unwrap_err();
    assert_eq!(err as u32, ACCES);
    as_caller(1001, fs.setattr(id, sattr3::default()))
        .await
        .expect("owner setattr still works (and stays a no-op)");
}

/// The default agent (`/index`) remains public under Enforced — the auth
/// contract from init-pro-5 is unchanged by the metadata gate.
#[tokio::test]
async fn default_agent_index_metadata_stays_public() {
    let fs = make().await.with_auth(enforced());
    let root = fs.root_dir();
    let index_dir = fs.lookup(root, &nm(b"index")).await.unwrap();
    let id = fs.lookup(index_dir, &nm(b"shared")).await.unwrap();
    let text = b"public knowledge";
    fs.write(id, 0, text).await.unwrap();

    // Any caller, even unregistered ones, may stat the default agent's files.
    for uid in [0, 1001, 1002, 4242] {
        let attr = as_caller(uid, fs.getattr(id)).await.unwrap();
        assert_eq!(attr.size, text.len() as u64, "uid {uid} reads default-agent size");
    }
}

/// Directories, search/where virtual files and the real FS are NOT
/// intercepted — the narrowed init-pro-5 D4 exception (protects NFS client
/// attribute caches from spurious ACCES on paths that carry no agent data).
#[tokio::test]
async fn dirs_search_where_and_real_fs_metadata_untouched() {
    let fs = make().await.with_auth(enforced());
    let root = fs.root_dir();

    // Real FS file: metadata readable by any caller.
    let (real_id, _) = fs.create(root, &nm(b"plain.txt"), sattr3::default()).await.unwrap();
    for uid in [0, 1002] {
        as_caller(uid, fs.getattr(real_id)).await.expect("real FS metadata stays public");
    }

    // Magic dirs + search/where virtual files: public virtual attributes.
    for name in ["search", "index", "where", "agents"] {
        let dir = as_caller(1002, fs.lookup(root, &nm(name.as_bytes()))).await.unwrap();
        as_caller(1002, fs.getattr(dir))
            .await
            .expect("magic dir metadata stays public");
    }
    let search_dir = fs.lookup(root, &nm(b"search")).await.unwrap();
    let q = fs.lookup(search_dir, &nm(b"anything")).await.unwrap();
    as_caller(1002, fs.getattr(q))
        .await
        .expect("virtual search file metadata stays public");
}

/// Stale/unresolvable CAT_INDEX fileids keep their pre-init-pro-7 shape:
/// zero-sized regular-file attrs, no ACCES — restart semantics (TODO-37/38)
/// are unchanged.
#[tokio::test]
async fn unresolvable_index_fileid_reports_zero_size_not_acces() {
    let fs = make().await.with_auth(enforced());
    let ghost = anyhome::magic::magic_file_id(
        anyhome::magic::CAT_INDEX,
        &anyhome::magic::scoped_name("botA", "never-looked-up"),
    );
    let attr = as_caller(1002, fs.getattr(ghost))
        .await
        .expect("no registry entry -> no agent data to protect");
    assert_eq!(attr.size, 0);
    assert!(matches!(attr.ftype, nfsserve::nfs::ftype3::NF3REG));
}

/// Open policy (no credentials configured): metadata behavior identical to
/// init-pro-6 — the backward-compatibility collapse point.
#[tokio::test]
async fn open_policy_metadata_backcompat() {
    let fs = make().await; // no with_auth -> AuthPolicy::Open
    let id = as_caller(9999, agent_file(&fs, "botX", "index", "k")).await;
    as_caller(9999, fs.write(id, 0, b"hello")).await.unwrap();

    // Every caller sees the real size; nothing is gated.
    for uid in [0, 1, 9999] {
        let attr = as_caller(uid, fs.getattr(id)).await.unwrap();
        assert_eq!(attr.size, 5);
    }
    as_caller(1, fs.setattr(id, sattr3::default()))
        .await
        .expect("setattr ungated under Open");
}

/// `/agents/<id>/search_k<N>/<q>` inherits the agent gate at the container
/// and scopes the KNN to the agent, with the explicit top-k applied.
#[tokio::test]
async fn agent_scoped_search_k_gated_and_scoped() {
    let fs = make().await.with_auth(enforced());
    let root = fs.root_dir();

    // botA indexes two docs mentioning "shared".
    let a_index = as_caller(1001, agent_file(&fs, "botA", "index", "a1")).await;
    as_caller(1001, fs.write(a_index, 0, b"shared one")).await.unwrap();
    let a_index2 = as_caller(1001, agent_file(&fs, "botA", "index", "a2")).await;
    as_caller(1001, fs.write(a_index2, 0, b"shared two")).await.unwrap();

    // botB cannot even enter /agents/botA (existing gate) ...
    let agents = fs.lookup(root, &nm(b"agents")).await.unwrap();
    let err = as_caller(1002, fs.lookup(agents, &nm(b"botA"))).await.unwrap_err();
    assert_eq!(err as u32, ACCES);

    // ... so the search_k dir under it is unreachable for botB by path.
    let a_dir = as_caller(1001, fs.lookup(agents, &nm(b"botA"))).await.unwrap();
    let k1 = as_caller(1001, fs.lookup(a_dir, &nm(b"search_k1"))).await.unwrap();
    let q = as_caller(1001, fs.lookup(k1, &nm(b"shared"))).await.unwrap();
    let (data, _) = as_caller(1001, fs.read(q, 0, 4096)).await.unwrap();
    let body = String::from_utf8_lossy(&data);
    assert_eq!(body.lines().count(), 1, "k=1 within agent scope: {body:?}");

    // `..` from the agent-scoped k-dir resolves to the agent container dir.
    let up = as_caller(1001, fs.lookup(k1, &nm(b".."))).await.unwrap();
    assert_eq!(up, a_dir);

    // Global `/search_k<N>` `..` resolves to the export root.
    let gk = fs.lookup(root, &nm(b"search_k4")).await.unwrap();
    assert_eq!(fs.lookup(gk, &nm(b"..")).await.unwrap(), root);
}
