//! anyhome CLI: assemble the layers and serve NFSv3.

use std::sync::Arc;

use anyhow::{Context, Result};
use clap::Parser;
use nfsserve::tcp::{NFSTcp, NFSTcpListener};

use anyhome::cli::{port_of, EmbedderKind};
use anyhome::db::{DummyEmbedder, Embedder};
use anyhome::{AnyHomeFs, Cli, MemoryDb, RealFs};
#[cfg(feature = "embed-fastembed")]
use anyhome::FastEmbedder;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "anyhome=info,nfsserve=warn".into()),
        )
        .init();

    let cli = Cli::parse();

    if !cli.fs_root.exists() {
        std::fs::create_dir_all(&cli.fs_root)?;
    }

    // Pick the embedder first: its dimensionality dictates the DB schema (the
    // embedding column is FLOAT32(dim)), and MemoryDb validates dim on reopen.
    let (embedder, dim) = build_embedder(&cli)?;
    tracing::info!(
        fs_root = ?cli.fs_root,
        db_path = %cli.db_path,
        addr = %cli.addr,
        export = %cli.export,
        embedder = ?cli.embedder,
        dim,
        "starting anyhome",
    );

    let real = RealFs::new(cli.fs_root.clone())?;
    let db = MemoryDb::open(&cli.db_path, dim).await?;
    // Clone for the graceful-shutdown checkpoint below; `db` itself moves into
    // AnyHomeFs (MemoryDb is cheaply Clone — shares the one connection).
    let db_shutdown = db.clone();

    // Assemble the agent authorization policy from `--agent-uid` flags and an
    // optional `--agent-creds` file. With no credentials at all the policy
    // collapses to Open (auth disabled) — backward compatible.
    let policy = anyhome::auth::build_auth_policy(&cli)?;
    let fs = AnyHomeFs::new(real, db, embedder).with_auth(policy);

    // Wire the export name into nfsserve so MOUNT enforces it: a client must
    // request exactly `host:<export>` or it gets MNT3ERR_NOENT.
    let mut listener = NFSTcpListener::bind(&cli.addr, fs).await?;
    listener.with_export_name(&cli.export);

    tracing::info!(
        addr = %cli.addr,
        port = port_of(&cli.addr),
        export = %cli.export,
        "NFSv3 listening. mount: mount.nfs -o vers=3,tcp,nolock,noacl,port={p},mountport={p} localhost:{e} /mnt/anyhome",
        p = port_of(&cli.addr),
        e = if cli.export == "/" { "/".to_string() } else { cli.export.clone() },
    );
    let mut sigint = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::interrupt())
        .context("install SIGINT handler")?;
    let mut sigterm = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        .context("install SIGTERM handler")?;

    let served = tokio::select! {
        res = listener.handle_forever() => Some(res),
        _ = sigint.recv() => { tracing::info!("SIGINT received: starting graceful shutdown"); None }
        _ = sigterm.recv() => { tracing::info!("SIGTERM received: starting graceful shutdown"); None }
    };
    if let Some(res) = served {
        res.context("nfs listener")?;
    }

    // Converge the WAL into the main DB file so a plain copy of `--db-path` is a
    // complete backup. Committed writes are already fsynced (synchronous=FULL), so
    // a checkpoint failure loses nothing — it only leaves data in the WAL.
    match db_shutdown.checkpoint().await {
        Ok((0, wal_frames, checkpointed)) => {
            tracing::info!(wal_frames, checkpointed, "WAL checkpoint complete (TRUNCATE)");
        }
        Ok((busy, wal_frames, checkpointed)) => {
            tracing::warn!(busy, wal_frames, checkpointed,
                "WAL checkpoint blocked by a concurrent reader (PASSIVE fallback); WAL retains frames");
        }
        Err(e) => {
            tracing::warn!(error = %e, "WAL checkpoint failed; committed data remains safe in the WAL");
        }
    }
    Ok(())
}

/// Resolve the configured embedder and the dimension it implies.
///
/// `dummy` keeps the build dependency-free; `fastembed` (feature-gated) loads
/// the real ONNX model and pins dim to the model's dim (384), overriding any
/// explicit `--dim`.
fn build_embedder(cli: &Cli) -> Result<(Arc<dyn Embedder>, usize)> {
    match cli.embedder {
        EmbedderKind::Dummy => {
            tracing::warn!(
                "using DummyEmbedder -- /search does exact-match only, no real semantics. \
                 Build with --features embed-fastembed and pass --embedder fastembed for real semantics."
            );
            Ok((Arc::new(DummyEmbedder::new(cli.dim.max(1))), cli.dim.max(1)))
        }
        EmbedderKind::Fastembed => {
            #[cfg(feature = "embed-fastembed")]
            {
                let e = FastEmbedder::new()?;
                let dim = e.dim();
                if cli.dim != dim {
                    tracing::warn!("--dim {} ignored: fastembed pins dim to {}", cli.dim, dim);
                }
                Ok((Arc::new(e) as Arc<dyn Embedder>, dim))
            }
            #[cfg(not(feature = "embed-fastembed"))]
            {
                let _ = cli.dim;
                anyhow::bail!(
                    "--embedder fastembed requires building with --features embed-fastembed"
                );
            }
        }
    }
}