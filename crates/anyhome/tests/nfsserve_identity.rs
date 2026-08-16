//! Ported line-for-line from the upstream nfsserve inline tests at the bottom
//! of `src/identity.rs` (`link_repos/nfsserve/src/identity.rs`, `mod tests`),
//! re-hosted here as anyhome integration tests. They lock down the task-local
//! caller-identity contract that anyhome `src/auth.rs` depends on: outside any
//! RPC, `current_identity()` returns the anonymous `Default`; inside a
//! `CALLER.scope`, the scoped identity is visible. TODO-57 / init-pro-10.

use nfsserve::identity::{current_identity, CallerIdentity, CALLER};

/// Outside any RPC, `current_identity()` yields the anonymous default identity.
#[tokio::test]
async fn defaults_outside_server() {
    let id = current_identity();
    assert!(id.is_anonymous());
    assert_eq!(id.uid, 0);
    assert_eq!(id.gid, 0);
    assert!(id.machinename.is_empty());
    assert!(id.client_addr.is_empty());
}

/// Inside `CALLER.scope`, the scoped identity is visible to `current_identity()`.
#[tokio::test]
async fn scoped_identity_visible() {
    let me = CallerIdentity {
        uid: 1001,
        gid: 1001,
        machinename: b"hostA".to_vec(),
        client_addr: "127.0.0.1:9000".to_string(),
    };
    let seen = CALLER.scope(me.clone(), async { current_identity() }).await;
    assert_eq!(seen.uid, 1001);
    assert_eq!(seen.machinename, b"hostA");
    assert!(!seen.is_anonymous());
}
