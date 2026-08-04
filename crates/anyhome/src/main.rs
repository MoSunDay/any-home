//! anyhome CLI: assemble the layers and serve NFSv3.

use std::sync::Arc;

use anyhow::Result;
use clap::Parser;
use nfsserve::tcp::{NFSTcp, NFSTcpListener};

use anyhome::cli::{port_of, EmbedderKind};
use anyhome::db::{DummyEmbedder, Embedder};
use anyhome::{AnyHomeFs, AuthPolicy, Cli, MemoryDb, RealFs};
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

    // Assemble the agent authorization policy from `--agent-uid` flags and an
    // optional `--agent-creds` file. With no credentials at all the policy
    // collapses to Open (auth disabled) — backward compatible.
    let policy = build_auth_policy(&cli)?;
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
    listener.handle_forever().await?;
    Ok(())
}

/// Build the agent authorization policy from CLI credential sources.
///
/// Merges repeated `--agent-uid <id:uid>` flags with an optional
/// `--agent-creds <file>`. An empty credential set yields [`AuthPolicy::Open`]
/// (auth disabled). Duplicate `(agent_id, uid)` bindings are de-duplicated by
/// the policy's set semantics.
fn build_auth_policy(cli: &Cli) -> Result<AuthPolicy> {
    let mut creds = cli.agent_uid.clone();
    if let Some(path) = &cli.agent_creds {
        let file_creds = anyhome::auth::parse_creds_file(path)
            .map_err(|e| anyhow::anyhow!("--agent-creds {path:?}: {e}"))?;
        creds.extend(file_creds);
    }
    if creds.is_empty() {
        tracing::info!("no agent credentials configured — authorization disabled (Open policy)");
    } else {
        tracing::info!(
            "authorization enabled: {} agent credential binding(s)",
            creds.len()
        );
    }
    Ok(AuthPolicy::from_creds(&creds))
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