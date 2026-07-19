//! anyhome CLI: assemble the layers and serve NFSv3.

use std::sync::Arc;

use anyhow::Result;
use clap::Parser;
use nfsserve::tcp::{NFSTcp, NFSTcpListener};

use anyhome::cli::port_of;
use anyhome::db::{DummyEmbedder, Embedder};
use anyhome::{AnyHomeFs, Cli, MemoryDb, RealFs};

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "anyhome=info,nfsserve=warn".into()),
        )
        .init();

    let cli = Cli::parse();
    tracing::info!(
        fs_root = ?cli.fs_root,
        db_path = %cli.db_path,
        addr = %cli.addr,
        export = %cli.export,
        dim = cli.dim,
        "starting anyhome",
    );

    if !cli.fs_root.exists() {
        std::fs::create_dir_all(&cli.fs_root)?;
    }
    let real = RealFs::new(cli.fs_root.clone())?;
    let db = MemoryDb::open(&cli.db_path, cli.dim).await?;

    // TODO-04 (real semantic embedder) is undecided; default to the
    // dependency-free DummyEmbedder and warn loudly.
    tracing::warn!(
        "TODO-04 undecided: using DummyEmbedder — /search does exact-match only, no real semantics yet"
    );
    let embedder: Arc<dyn Embedder> = Arc::new(DummyEmbedder::new(cli.dim));

    let fs = AnyHomeFs::new(real, db, embedder);
    let listener = NFSTcpListener::bind(&cli.addr, fs).await?;
    tracing::info!(
        addr = %cli.addr,
        port = port_of(&cli.addr),
        export = %cli.export,
        "NFSv3 listening. mount: mount.nfs -o vers=3,tcp,nolock,noacl,port={p},mountport={p} localhost:{e} /mnt/anyhome",
        p = port_of(&cli.addr),
        e = cli.export,
    );
    listener.handle_forever().await?;
    Ok(())
}
