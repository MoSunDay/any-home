//! CLI definition, shared between the `anyhome` binary and tests.

use std::path::PathBuf;

use clap::Parser;

/// Which embedding backend to use.
#[derive(Parser, Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum EmbedderKind {
    /// Dependency-free deterministic embedder (exact-match semantics only).
    #[default]
    Dummy,
    /// Local ONNX semantic embedder (requires the `embed-fastembed` feature).
    Fastembed,
}

impl std::fmt::Display for EmbedderKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            EmbedderKind::Dummy => "dummy",
            EmbedderKind::Fastembed => "fastembed",
        };
        f.write_str(s)
    }
}

impl std::str::FromStr for EmbedderKind {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_ascii_lowercase().as_str() {
            "dummy" => Ok(EmbedderKind::Dummy),
            "fastembed" => Ok(EmbedderKind::Fastembed),
            other => Err(format!(
                "unknown embedder '{other}' (expected 'dummy' or 'fastembed')"
            )),
        }
    }
}

#[derive(Parser, Debug)]
#[command(name = "anyhome", version, about = "Mountable NFSv3 memory service")]
pub struct Cli {
    /// On-disk directory backing the filesystem layer.
    #[arg(long, default_value = "./anyhome-fs")]
    pub fs_root: PathBuf,

    /// libSQL database path (":memory:" for an ephemeral DB).
    #[arg(long, default_value = ":memory:")]
    pub db_path: String,

    /// Listen address (ip:port). NFS/MOUNT/portmap multiplex on this one port.
    #[arg(long, default_value = "127.0.0.1:11111")]
    pub addr: String,

    /// Export path surfaced to NFS clients (e.g. `/mem`; default `/`).
    /// Wired into nfsserve so `mount host:/mem` is enforced by MOUNT.
    #[arg(long, default_value = "/")]
    pub export: String,

    /// Embedding backend.
    ///
    /// - `dummy` (default): deterministic, no dependencies; `/search` is
    ///   exact-match only.
    /// - `fastembed`: real semantic embeddings (all-MiniLM-L6-v2, dim 384);
    ///   requires building with `--features embed-fastembed`. With `fastembed`,
    ///   `--dim` is forced to the model's dim (384) regardless of this flag.
    #[arg(long, default_value_t)]
    pub embedder: EmbedderKind,

    /// Embedding dimension for the `dummy` embedder (ignored when `--embedder
    /// fastembed`, which pins dim=384). Must match any pre-existing db file.
    #[arg(long, default_value_t = crate::db::DUMMY_DIM)]
    pub dim: usize,

    /// Register an agent credential, binding `agent_id` to a caller AUTH_SYS
    /// `uid` (`--agent-uid botA:1001`). Repeatable. When at least one credential
    /// is registered (here or via `--agent-creds`) authorization is *enabled*:
    /// a caller's uid must be bound to an agent_id to access
    /// `/agents/<id>/...`. With no credentials configured at all, auth is
    /// disabled (identical to prior behavior). Soft auth only — see README.
    #[arg(long, value_name = "ID:UID", value_parser = crate::auth::AgentCred::parse_clap)]
    pub agent_uid: Vec<crate::auth::AgentCred>,

    /// Path to a credentials file (one `<agent_id>:<uid>` per line; `#`
    /// comments and blank lines ignored). Merged with `--agent-uid`. See
    /// `--agent-uid` for the auth model.
    #[arg(long, value_name = "FILE")]
    pub agent_creds: Option<PathBuf>,
}

/// Extract the port portion of an `ip:port` string (for log lines).
pub fn port_of(addr: &str) -> String {
    addr.rsplit(':').next().unwrap_or("?").to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cli_accepts_all_flags() {
        let cli = Cli::try_parse_from([
            "anyhome",
            "--fs-root", "/tmp/x",
            "--db-path", ":memory:",
            "--addr", "127.0.0.1:11111",
            "--export", "/",
            "--embedder", "dummy",
            "--dim", "8",
        ])
        .expect("all flags parse");
        assert_eq!(cli.fs_root, PathBuf::from("/tmp/x"));
        assert_eq!(cli.db_path, ":memory:");
        assert_eq!(cli.addr, "127.0.0.1:11111");
        assert_eq!(cli.export, "/");
        assert_eq!(cli.embedder, EmbedderKind::Dummy);
        assert_eq!(cli.dim, 8);
    }

    #[test]
    fn cli_accepts_embedder_fastembed_flag() {
        // End-to-end CLI parse of the new --embedder flag, exercising clap's
        // value_parser (FromStr) dispatch rather than calling FromStr directly.
        let cli = Cli::try_parse_from([
            "anyhome", "--embedder", "fastembed",
        ])
        .expect("--embedder fastembed parses");
        assert_eq!(cli.embedder, EmbedderKind::Fastembed);
    }

    #[test]
    fn cli_defaults_are_sane() {
        let cli = Cli::try_parse_from(["anyhome"]).unwrap();
        assert_eq!(cli.addr, "127.0.0.1:11111");
        assert_eq!(cli.export, "/");
        assert_eq!(cli.embedder, EmbedderKind::Dummy);
        assert_eq!(cli.dim, crate::db::DUMMY_DIM);
    }

    #[test]
    fn embedder_kind_fromstr_parses_known_backends() {
        // Normal paths: both variants, plus case-insensitivity (the impl
        // lower-cases its input).
        assert_eq!("dummy".parse::<EmbedderKind>().unwrap(), EmbedderKind::Dummy);
        assert_eq!("fastembed".parse::<EmbedderKind>().unwrap(), EmbedderKind::Fastembed);
        assert_eq!("DUMMY".parse::<EmbedderKind>().unwrap(), EmbedderKind::Dummy);
        assert_eq!("FastEmbed".parse::<EmbedderKind>().unwrap(), EmbedderKind::Fastembed);
        // Display round-trips the canonical lowercase spelling.
        assert_eq!(EmbedderKind::Dummy.to_string(), "dummy");
        assert_eq!(EmbedderKind::Fastembed.to_string(), "fastembed");
    }

    #[test]
    fn embedder_kind_fromstr_rejects_unknown() {
        // Error path: unknown value yields an Err whose message names the
        // offending value and the accepted set.
        let err = "bogus".parse::<EmbedderKind>().expect_err("unknown embedder must error");
        let msg = err.to_string();
        assert!(msg.contains("unknown embedder 'bogus'"), "got: {msg}");
        assert!(msg.contains("dummy"), "got: {msg}");
        assert!(msg.contains("fastembed"), "got: {msg}");
    }

    #[test]
    fn port_of_works() {
        // Well-formed ip:port -> the port segment.
        assert_eq!(port_of("1.2.3.4:11111"), "11111");
        // No colon: rsplit(':') yields the whole string as its only segment,
        // so the entire input is returned (the unwrap_or("?") fallback only
        // guards an empty iterator, which rsplit never produces). This
        // characterizes the actual fallback semantics for log-line callers.
        assert_eq!(port_of("nope"), "nope");
        // Trailing colon -> empty port segment.
        assert_eq!(port_of("host:"), "");
    }
}
