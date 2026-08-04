//! Agent identity authorization integration tests (TODO-28..33, init-pro-5).
//!
//! Drives the dispatcher's NFSFileSystem surface while simulating distinct
//! AUTH_SYS callers via `nfsserve::identity::CALLER.scope(...)`. Covers:
//! - backward compatibility (Open policy = no checks),
//! - impersonation blocked at `/agents/<id>` lookup,
//! - cross-agent read/write denied even with a leaked file handle,
//! - anonymous callers confined to the default agent,
//! - the default agent (`/index`, `/search`, `/where`) stays public,
//! - `/agents` readdir hides agents the caller does not own,
//! - one uid owning several agents.

use std::future::Future;
use std::sync::Arc;

use nfsserve::identity::{CallerIdentity, CALLER};
use nfsserve::nfs::{fileid3, filename3, nfsstat3};
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

/// Resolve `/agents/<agent>/<sub>/<name>` from root. Caller must already be
/// authorized for `agent` (wrap the whole call in [`as_caller`]).
async fn agent_file(fs: &AnyHomeFs, agent: &str, sub: &str, name: &str) -> fileid3 {
    let root = fs.root_dir();
    let agents = fs.lookup(root, &nm(b"agents")).await.unwrap();
    let a_dir = fs.lookup(agents, &nm(agent.as_bytes())).await.unwrap();
    let sub_dir = fs.lookup(a_dir, &nm(sub.as_bytes())).await.unwrap();
    fs.lookup(sub_dir, &nm(name.as_bytes())).await.unwrap()
}

#[tokio::test]
async fn open_policy_allows_any_caller_any_agent() {
    // No credentials -> Open policy: every caller may act as any agent
    // (identical to init-pro-4). This is the backward-compatibility contract.
    let fs = make().await;
    let id = as_caller(9999, agent_file(&fs, "botX", "index", "k")).await;
    as_caller(9999, fs.write(id, 0, b"hello")).await.unwrap();
    assert_eq!(fs.db.list_agents().await.unwrap(), vec!["botX".to_string()]);
}

#[tokio::test]
async fn enforced_impersonation_blocked_at_lookup() {
    let fs = make()
        .await
        .with_auth(AuthPolicy::from_creds(&[
            AgentCred::new("botA", 1001),
            AgentCred::new("botB", 1002),
        ]));

    let root = fs.root_dir();
    let agents = as_caller(1001, fs.lookup(root, &nm(b"agents"))).await.unwrap();

    // botA (uid 1001) may enter its own namespace...
    as_caller(1001, fs.lookup(agents, &nm(b"botA")))
        .await
        .expect("owner can enter own namespace");
    // ...but impersonating botB is denied at the lookup gate.
    let err = as_caller(1001, fs.lookup(agents, &nm(b"botB")))
        .await
        .unwrap_err();
    assert_eq!(err as u32, ACCES);
}

#[tokio::test]
async fn enforced_cross_agent_write_and_read_denied_with_leaked_handle() {
    // NFS clients cache file handles. Once a handle leaks, a later write/read
    // must still be gated by the recovered agent_id, not just the lookup path.
    let fs = make()
        .await
        .with_auth(AuthPolicy::from_creds(&[
            AgentCred::new("botA", 1001),
            AgentCred::new("botB", 1002),
        ]));

    // botA legitimately resolves and registers its index file handle.
    let id = as_caller(1001, agent_file(&fs, "botA", "index", "secret")).await;

    // botB attempts to write through that same handle -> denied.
    let err = as_caller(1002, fs.write(id, 0, b"pwned"))
        .await
        .unwrap_err();
    assert_eq!(err as u32, ACCES);

    // And cannot read it either.
    let err = as_caller(1002, fs.read(id, 0, 4096))
        .await
        .unwrap_err();
    assert_eq!(err as u32, ACCES);

    // botA itself can still write/read fine.
    as_caller(1001, fs.write(id, 0, b"mine")).await.unwrap();
    let (data, _) = as_caller(1001, fs.read(id, 0, 4096)).await.unwrap();
    assert!(data.starts_with(b"mine"));
}

#[tokio::test]
async fn enforced_cross_agent_namespace_lookup_denied() {
    let fs = make()
        .await
        .with_auth(AuthPolicy::from_creds(&[
            AgentCred::new("botA", 1001),
            AgentCred::new("botB", 1002),
        ]));

    let root = fs.root_dir();
    let agents = as_caller(1002, fs.lookup(root, &nm(b"agents")))
        .await
        .unwrap();
    // botB cannot even step into botA's namespace.
    let err = as_caller(1002, fs.lookup(agents, &nm(b"botA")))
        .await
        .unwrap_err();
    assert_eq!(err as u32, ACCES);
}

#[tokio::test]
async fn anonymous_caller_confined_to_default_agent() {
    // uid 0 = anonymous (no AUTH_SYS credential). It may use the global
    // (default-agent) paths but cannot enter any named agent namespace.
    let fs = make()
        .await
        .with_auth(AuthPolicy::from_creds(&[AgentCred::new("botA", 1001)]));

    // Global /index/<key> (default agent) works for the anonymous caller.
    let root = fs.root_dir();
    let idx = as_caller(0, fs.lookup(root, &nm(b"index"))).await.unwrap();
    let gid = as_caller(0, fs.lookup(idx, &nm(b"publickey"))).await.unwrap();
    as_caller(0, fs.write(gid, 0, b"world-readable")).await.unwrap();

    // But it cannot impersonate botA.
    let agents = as_caller(0, fs.lookup(root, &nm(b"agents")))
        .await
        .unwrap();
    let err = as_caller(0, fs.lookup(agents, &nm(b"botA")))
        .await
        .unwrap_err();
    assert_eq!(err as u32, ACCES);
}

#[tokio::test]
async fn default_agent_paths_remain_public_under_enforcement() {
    // The default agent (agent_id = "") is always public, so the global
    // /search, /index, /where paths keep working for every caller even when
    // auth is enforced.
    let fs = make()
        .await
        .with_auth(AuthPolicy::from_creds(&[AgentCred::new("botA", 1001)]));

    let root = fs.root_dir();
    for uid in [0_u32, 9999] {
        let idx = as_caller(uid, fs.lookup(root, &nm(b"index")))
            .await
            .unwrap();
        let id = as_caller(uid, fs.lookup(idx, &nm(b"shared")))
            .await
            .unwrap();
        as_caller(uid, fs.write(id, 0, b"data")).await.unwrap();
        let (data, _) = as_caller(uid, fs.read(id, 0, 4096))
            .await
            .unwrap();
        assert!(data.windows(4).any(|w| w == b"data"));
    }
}

/// Names visible in `/agents` for `uid` under the current policy.
async fn visible_agents(fs: &AnyHomeFs, uid: u32) -> Vec<String> {
    let root = fs.root_dir();
    let agents = as_caller(uid, fs.lookup(root, &nm(b"agents")))
        .await
        .unwrap();
    let listing = as_caller(uid, fs.readdir(agents, 0, 64))
        .await
        .unwrap();
    let mut names: Vec<String> = listing
        .entries
        .iter()
        .map(|e| String::from_utf8_lossy(&e.name).into_owned())
        .collect();
    names.sort();
    names
}

#[tokio::test]
async fn readdir_agents_hides_unauthorized_namespaces() {
    // `/agents` readdir filters to only agents the caller may access, so a
    // caller cannot even discover other agents' names.
    let fs = make()
        .await
        .with_auth(AuthPolicy::from_creds(&[
            AgentCred::new("botA", 1001),
            AgentCred::new("botB", 1002),
        ]));

    // Seed both agents (each owner writes one doc).
    let a = as_caller(1001, agent_file(&fs, "botA", "index", "k")).await;
    as_caller(1001, fs.write(a, 0, b"a")).await.unwrap();
    let b = as_caller(1002, agent_file(&fs, "botB", "index", "k")).await;
    as_caller(1002, fs.write(b, 0, b"b")).await.unwrap();

    assert_eq!(visible_agents(&fs, 1001).await, vec!["botA".to_string()]);
    assert_eq!(visible_agents(&fs, 1002).await, vec!["botB".to_string()]);
}

#[tokio::test]
async fn one_uid_owns_multiple_agents() {
    // A supervisor uid may be bound to several agent namespaces.
    let fs = make()
        .await
        .with_auth(AuthPolicy::from_creds(&[
            AgentCred::new("botA", 1001),
            AgentCred::new("botB", 1001),
        ]));

    let root = fs.root_dir();
    let agents = as_caller(1001, fs.lookup(root, &nm(b"agents")))
        .await
        .unwrap();
    as_caller(1001, fs.lookup(agents, &nm(b"botA")))
        .await
        .unwrap();
    as_caller(1001, fs.lookup(agents, &nm(b"botB")))
        .await
        .unwrap();
    // Another uid cannot reach either.
    let err = as_caller(2000, fs.lookup(agents, &nm(b"botA")))
        .await
        .unwrap_err();
    assert_eq!(err as u32, ACCES);
}

#[tokio::test]
async fn enforced_remove_denied_for_non_owner() {
    let fs = make()
        .await
        .with_auth(AuthPolicy::from_creds(&[
            AgentCred::new("botA", 1001),
            AgentCred::new("botB", 1002),
        ]));

    // botA writes a memory.
    let id = as_caller(1001, agent_file(&fs, "botA", "index", "k")).await;
    as_caller(1001, fs.write(id, 0, b"v")).await.unwrap();

    // Recover botA's index dir handle as the owner.
    let root = fs.root_dir();
    let agents = as_caller(1001, fs.lookup(root, &nm(b"agents")))
        .await
        .unwrap();
    let a_dir = as_caller(1001, fs.lookup(agents, &nm(b"botA")))
        .await
        .unwrap();
    let idx_dir = as_caller(1001, fs.lookup(a_dir, &nm(b"index")))
        .await
        .unwrap();

    // botB tries to remove botA's key through botA's index dir handle.
    let err = as_caller(1002, fs.remove(idx_dir, &nm(b"k")))
        .await
        .unwrap_err();
    assert_eq!(err as u32, ACCES);

    // botA can remove its own key.
    as_caller(1001, fs.remove(idx_dir, &nm(b"k")))
        .await
        .unwrap();
}
