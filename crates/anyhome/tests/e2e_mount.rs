//! Local-mount e2e, in Rust: spawn the real `anyhome` binary, mount it with
//! the kernel NFSv3 client (`mount.nfs`), and assert the magic-path contract
//! over the wire — the cargo-integrated twin of `scripts/e2e.sh`'s core
//! facets, so `cargo test` exercises a REAL mount whenever the host allows.
//!
//! Skips (prints, passes) when the host cannot mount — `mount.nfs` missing or
//! not running as root — mirroring e2e.sh's default skip so NFS-less sandboxes
//! stay green. Set ANYHOME_REQUIRE_MOUNT=1 to turn an unmountable host into a
//! hard failure. Full live parity (auth/agent isolation, fastembed) stays
//! with `scripts/e2e.sh --require-mount`; this file pins the core loop:
//! real-file lifecycle, /index->/search, /where(+prefix), true-size,
//! overwrite/O_TRUNC, rm triple-check (incl. DB rows), unicode, /search_k,
//! export-name enforcement, and restart persistence after a graceful SIGTERM.

use std::fs::{self, File};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::time::{Duration, Instant};

use tempfile::TempDir;

type R = Result<(), String>;

const HOST: &str = "127.0.0.1";
const EXPORT: &str = "/";
// nfsserve multiplexes NFS+MOUNT+portmap on ONE port and runs no rpcbind/NLM,
// so the mount must pin vers/tcp/nolock/noacl + port == mountport (e2e.sh).
fn mount_opts(port: u16) -> String {
    format!("vers=3,tcp,nolock,noacl,port={port},mountport={port}")
}

// ---- tiny assertion/IO helpers ----------------------------------------------

fn ck(cond: bool, msg: &str) -> R {
    cond.then_some(()).ok_or_else(|| msg.to_string())
}

fn io<T>(r: std::io::Result<T>, what: &str) -> Result<T, String> {
    r.map_err(|e| format!("{what}: {e}"))
}

fn write_file(p: &Path, data: &str) -> R {
    io(fs::write(p, data), &format!("write {}", p.display()))
}

fn read_file(p: &Path) -> Result<String, String> {
    fs::read_to_string(p).map_err(|e| format!("read {}: {e}", p.display()))
}

fn has_line_starting(s: &str, prefix: &str) -> bool {
    s.lines().any(|l| l.starts_with(prefix))
}

fn dir_names(p: &Path) -> Result<Vec<String>, String> {
    let mut out = Vec::new();
    for e in fs::read_dir(p).map_err(|e| format!("readdir {}: {e}", p.display()))? {
        let e = e.map_err(|e| e.to_string())?;
        out.push(e.file_name().to_string_lossy().to_string());
    }
    Ok(out)
}

// ---- host capability / process / mount plumbing ------------------------------

/// The host can actually perform a kernel NFSv3 mount: Linux + mount.nfs + root.
fn mount_backend() -> Result<(), String> {
    if !cfg!(target_os = "linux") {
        return Err("kernel-NFS e2e is Linux-only".into());
    }
    let ok = Command::new("mount.nfs")
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    if !ok {
        return Err("mount.nfs not found (apt install nfs-common)".into());
    }
    let uid = Command::new("id")
        .arg("-u")
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default();
    if uid != "0" {
        return Err("mount.nfs requires root".into());
    }
    Ok(())
}

fn require_mount() -> bool {
    std::env::var("ANYHOME_REQUIRE_MOUNT").map(|v| v == "1").unwrap_or(false)
}

fn free_port() -> Result<u16, String> {
    let l = io(TcpListener::bind((HOST, 0)), "bind ephemeral port")?;
    Ok(io(l.local_addr(), "ephemeral local_addr")?.port())
}

fn spawn_server(bin: &str, fs_root: &Path, db: &Path, port: u16, log: &Path) -> Result<Child, String> {
    let out = io(File::create(log), "create server log")?;
    let err = io(out.try_clone(), "clone server log fd")?;
    Command::new(bin)
        .arg("--fs-root").arg(fs_root)
        .arg("--db-path").arg(db)
        .arg("--addr").arg(format!("{HOST}:{port}"))
        .arg("--export").arg(EXPORT)
        .arg("--embedder").arg("dummy")
        .stdout(Stdio::from(out))
        .stderr(Stdio::from(err))
        .spawn()
        .map_err(|e| format!("spawn {bin}: {e}"))
}

fn wait_port(port: u16, secs: u64) -> bool {
    let deadline = Instant::now() + Duration::from_secs(secs);
    while Instant::now() < deadline {
        if TcpStream::connect((HOST, port)).is_ok() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    false
}

/// Start a server on a free port (retries a fresh port on EADDRINUSE races).
fn start_server(
    bin: &str,
    fs_root: &Path,
    db: &Path,
    log: &Path,
    servers: &mut Vec<Child>,
) -> Result<u16, String> {
    for _ in 0..3 {
        let port = free_port()?;
        servers.push(spawn_server(bin, fs_root, db, port, log)?);
        if wait_port(port, 15) {
            return Ok(port);
        }
        let log_s = fs::read_to_string(log).unwrap_or_default();
        if !log_s.contains("Address already in use") {
            return Err(format!("server did not come up; log tail:\n{}", tail(&log_s)));
        }
        eprintln!("    -> port {port} raced (EADDRINUSE), retrying");
    }
    Err("no free port survived 3 attempts".into())
}

fn tail(s: &str) -> String {
    s.lines().rev().take(15).collect::<Vec<_>>().join("\n")
}

fn nfs_mount(port: u16, export: &str, dir: &Path) -> R {
    let opts = mount_opts(port);
    let st = io(
        Command::new("mount.nfs")
            .arg("-o").arg(&opts)
            .arg(format!("{HOST}:{export}"))
            .arg(dir)
            .status(),
        "run mount.nfs",
    )?;
    ck(
        st.success(),
        &format!("mount.nfs {HOST}:{export} -> {} failed ({st})", dir.display()),
    )
}

/// Best-effort unmount: force first, lazy as fallback (never fails the run).
fn umount(dir: &Path) {
    for flag in ["-f", "-l"] {
        if Command::new("umount").arg(flag).arg(dir).status().map(|s| s.success()).unwrap_or(false) {
            return;
        }
    }
}

/// SIGTERM + bounded wait, SIGKILL fallback. Returns the exit status if reaped.
fn term(child: &mut Child) -> Result<Option<ExitStatus>, String> {
    let pid = child.id().to_string();
    io(Command::new("kill").arg("-TERM").arg(&pid).status(), "kill -TERM")?;
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if let Some(st) = child.try_wait().map_err(|e| e.to_string())? {
            return Ok(Some(st));
        }
        if Instant::now() > deadline {
            let _ = child.kill();
            let _ = child.wait();
            return Ok(None);
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// DB residue after `rm` (the third leg of the triple-check, like e2e.sh's
/// python sqlite3 probe): (memories rows, FTS matches) for the deleted key.
fn db_residue(db: &Path) -> Result<(i64, i64), String> {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| e.to_string())?;
    rt.block_on(async {
        let handle = libsql::Builder::new_local(db.to_string_lossy().to_string())
            .build()
            .await
            .map_err(|e| format!("open db: {e}"))?;
        let conn = handle.connect().map_err(|e| format!("connect db: {e}"))?;
        let rows = scalar(&conn, "SELECT count(*) FROM memories WHERE agent_id='' AND key='delete-me'").await?;
        let fts = scalar(&conn, "SELECT count(*) FROM memories_fts WHERE memories_fts MATCH 'temp'").await?;
        Ok((rows, fts))
    })
}

async fn scalar(conn: &libsql::Connection, sql: &str) -> Result<i64, String> {
    let mut rows = conn.query(sql, ()).await.map_err(|e| format!("`{sql}`: {e}"))?;
    match rows.next().await.map_err(|e| e.to_string())? {
        Some(row) => row.get::<i64>(0).map_err(|e| e.to_string()),
        None => Err(format!("`{sql}`: no row")),
    }
}

// ---- the e2e flow ------------------------------------------------------------

fn flow(work: &TempDir, bin: &str, servers: &mut Vec<Child>, mounts: &mut Vec<PathBuf>) -> R {
    let root = work.path();
    let fs_root = root.join("fs");
    let db = root.join("mem.db");
    let mnt = root.join("mnt");
    for d in [&fs_root, &mnt, &root.join("mnt-bad")] {
        io(fs::create_dir_all(d), &format!("mkdir {}", d.display()))?;
    }

    eprintln!("==> starting anyhome (fs={} db={})", fs_root.display(), db.display());
    let port = start_server(bin, &fs_root, &db, &root.join("server.log"), servers)?;

    eprintln!("==> mounting {HOST}:{EXPORT} at {}", mnt.display());
    nfs_mount(port, EXPORT, &mnt)?;
    mounts.push(mnt.clone());

    eprintln!("==> MOUNT must refuse a wrong export name");
    let bad = io(
        Command::new("mount.nfs")
            .arg("-o").arg(mount_opts(port))
            .arg(format!("{HOST}:/definitely-not-the-export"))
            .arg(root.join("mnt-bad"))
            .status(),
        "run mount.nfs (bad export)",
    )?;
    ck(!bad.success(), "server accepted a mount for a wrong export name")?;

    // Real-file lifecycle: the backing layer, not a magic path.
    eprintln!("==> facet: plain file create/read/delete on the real FS");
    write_file(&mnt.join("plain.txt"), "plain file on real fs\n")?;
    ck(
        read_file(&mnt.join("plain.txt"))? == "plain file on real fs\n",
        "plain.txt readback differs",
    )?;
    ck(fs_root.join("plain.txt").exists(), "plain.txt not materialized in fs-root")?;
    io(fs::remove_file(mnt.join("plain.txt")), "rm plain.txt")?;
    ck(!fs_root.join("plain.txt").exists(), "plain.txt still on disk after rm")?;
    ck(
        !dir_names(&mnt)?.iter().any(|n| n == "plain.txt"),
        "plain.txt still listed after rm",
    )?;

    // /index -> /search round-trip. DummyEmbedder: exact match ranks top-1.
    eprintln!("==> facet: /index/greeting -> /search top-1");
    write_file(&mnt.join("index/greeting"), "hello anyhome\n")?;
    let search = read_file(&mnt.join("search/hello%20anyhome"))?;
    ck(
        search.lines().next() == Some("greeting\thello anyhome"),
        "expected top-1 'greeting\\thello anyhome' in /search",
    )?;

    // /where: FTS5 MATCH must surface every memory containing the term.
    eprintln!("==> facet: /where full-text surfaces both keys");
    write_file(&mnt.join("index/second"), "hello world\n")?;
    let where_hello = read_file(&mnt.join("where/hello"))?;
    ck(has_line_starting(&where_hello, "greeting\t"), "greeting missing from /where/hello")?;
    ck(has_line_starting(&where_hello, "second\t"), "second missing from /where/hello")?;

    // /where/key:<prefix> prefix filtering.
    eprintln!("==> facet: /where/key:gre prefix filter");
    let where_gre = read_file(&mnt.join("where/key:gre"))?;
    ck(has_line_starting(&where_gre, "greeting\t"), "greeting missing from /where/key:gre")?;
    ck(!has_line_starting(&where_gre, "second\t"), "prefix filter leaked 'second'")?;

    // True attr size: GETATTR must advertise the stored byte count (RFC 1813
    // clients zero-pad READs up to attr size), query files included.
    eprintln!("==> facet: true attr size (index key + query file)");
    write_file(&mnt.join("index/size-key"), "exact size payload\n")?;
    let payload = b"exact size payload\n".len() as u64;
    ck(io(fs::metadata(mnt.join("index/size-key")), "stat size-key")?.len() == payload,
        "index/size-key attr size != content length")?;
    ck(read_file(&mnt.join("index/size-key"))? == "exact size payload\n",
        "index/size-key readback differs")?;
    let where_gre_len = io(fs::metadata(mnt.join("where/key:gre")), "stat where/key:gre")?.len();
    ck(where_gre_len == read_file(&mnt.join("where/key:gre"))?.len() as u64,
        "where/key:gre attr size != readable bytes")?;
    ck(where_gre_len < 1_048_576, "query file still advertises the virtual 1 MiB size")?;

    // Overwrite semantics: shorter replacement drops the stale tail; O_TRUNC
    // (File::create = shell `>`) empties the memory.
    eprintln!("==> facet: overwrite + O_TRUNC leave no stale tail");
    write_file(&mnt.join("index/overwrite-key"), "ORIGINAL-CONTENT-MUCH-LONGER-THAN-THE-REPLACEMENT\n")?;
    write_file(&mnt.join("index/overwrite-key"), "short\n")?;
    ck(read_file(&mnt.join("index/overwrite-key"))? == "short\n",
        "overwrite kept stale tail bytes (SETATTR size=0 ignored)")?;
    ck(io(fs::metadata(mnt.join("index/overwrite-key")), "stat overwrite-key")?.len() == 6,
        "overwritten memory attr size must be the new length")?;
    io(File::create(mnt.join("index/overwrite-key")), "O_TRUNC overwrite-key")?;
    ck(read_file(&mnt.join("index/overwrite-key"))?.is_empty(), "`>` must truncate to empty")?;
    write_file(&mnt.join("index/overwrite-key"), "short\n")?; // re-checked post-restart

    // rm triple-check: NFS enumeration + /where prefix + DB/FTS rows.
    eprintln!("==> facet: rm removes the memory everywhere (ls + /where + DB)");
    write_file(&mnt.join("index/delete-me"), "temp memory to delete\n")?;
    io(fs::remove_file(mnt.join("index/delete-me")), "rm delete-me")?;
    ck(!dir_names(&mnt.join("index"))?.iter().any(|n| n == "delete-me"),
        "delete-me still listed under /index after rm")?;
    ck(read_file(&mnt.join("where/key:delete-me"))?.is_empty(),
        "/where/key:delete-me still returns rows after rm")?;
    let (rows, fts) = db_residue(&db)?;
    ck(rows == 0 && fts == 0, "delete-me left DB/FTS residue after rm")?;

    // Unicode round-trip: percent-encoded UTF-8 on the wire.
    eprintln!("==> facet: unicode memory + UTF-8 queries");
    write_file(&mnt.join("index/unicode-key"), "café 记忆 ünïcode\n")?;
    ck(read_file(&mnt.join("index/unicode-key"))? == "café 记忆 ünïcode\n",
        "unicode readback differs")?;
    let u_search = read_file(&mnt.join("search/caf%C3%A9%20%E8%AE%B0%E5%BF%86%20%C3%BCn%C3%AFcode"))?;
    ck(has_line_starting(&u_search, "unicode-key\t"), "UTF-8 /search did not find unicode-key")?;
    let u_where = read_file(&mnt.join("where/%E8%AE%B0%E5%BF%86"))?;
    ck(has_line_starting(&u_where, "unicode-key\t"), "UTF-8 /where did not find unicode-key")?;

    // /search_k<N>: k-limited KNN keeps the exact match and caps the rows.
    eprintln!("==> facet: /search_k2 caps results");
    let sk = read_file(&mnt.join("search_k2/hello%20anyhome"))?;
    ck(has_line_starting(&sk, "greeting\t"), "/search_k2 must still rank the exact match")?;
    ck(sk.lines().filter(|l| !l.is_empty()).count() <= 2,
        "/search_k2 returned more than 2 rows")?;

    // Restart persistence: graceful SIGTERM (exit 0 + checkpointed WAL), same
    // db-path, remount, everything still there.
    eprintln!("==> facet: SIGTERM graceful exit + restart persistence");
    umount(&mnt);
    let st = term(&mut servers[0])?;
    ck(matches!(st, Some(s) if s.success()), "server must exit 0 on SIGTERM (graceful shutdown)")?;
    let wal = root.join("mem.db-wal");
    if wal.exists() {
        ck(io(fs::metadata(&wal), "stat wal")?.len() == 0,
            "graceful shutdown must leave a checkpointed (0-byte) WAL")?;
    }
    servers.push(spawn_server(bin, &fs_root, &db, port, &root.join("server.log"))?);
    ck(wait_port(port, 15), "server did not restart")?;
    nfs_mount(port, EXPORT, &mnt)?;

    ck(read_file(&mnt.join("index/overwrite-key"))? == "short\n",
        "post-restart readback of overwrite-key must be the truncated text")?;
    let after = read_file(&mnt.join("where/hello"))?;
    ck(has_line_starting(&after, "greeting\t"), "greeting missing from /where after restart")?;
    ck(has_line_starting(&after, "second\t"), "second missing from /where after restart")?;
    ck(read_file(&mnt.join("where/key:delete-me"))?.is_empty(), "delete-me resurrected after restart")?;

    eprintln!("PASS: local mount e2e (rust) — real fs + index/search/where(+prefix) + true-size + overwrite/truncate + rm triple-check + unicode + search_k + export-name + SIGTERM persistence all green over NFSv3");
    Ok(())
}

#[test]
fn local_mount_e2e() {
    if let Err(reason) = mount_backend() {
        if require_mount() {
            panic!("ANYHOME_REQUIRE_MOUNT=1 but host cannot mount: {reason}");
        }
        eprintln!("skipped: {reason} (set ANYHOME_REQUIRE_MOUNT=1 to enforce)");
        return;
    }
    let bin = env!("CARGO_BIN_EXE_anyhome");
    let work = tempfile::tempdir().expect("tempdir");
    let mut servers: Vec<Child> = Vec::new();
    let mut mounts: Vec<PathBuf> = Vec::new();
    let outcome = flow(&work, bin, &mut servers, &mut mounts);
    // Deterministic cleanup regardless of outcome: unmount first, then stop
    // the servers, so the TempDir drop can actually remove everything.
    for m in &mounts {
        umount(m);
    }
    for c in servers.iter_mut() {
        let _ = c.kill();
        let _ = c.wait();
    }
    outcome.expect("local mount e2e failed");
}
