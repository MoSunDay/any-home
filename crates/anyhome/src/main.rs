//! anyhome CLI: assemble the layers and serve NFSv3.

use std::sync::Arc;

use anyhow::Result;
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
    let fs = AnyHomeFs::new(real, db, embedder);

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
    listener.handle_forever().await?;
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