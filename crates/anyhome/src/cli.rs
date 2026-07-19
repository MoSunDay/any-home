//! CLI definition, shared between the `anyhome` binary and tests.

use std::path::PathBuf;

use clap::Parser;

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

    /// Export path (advisory; nfsserve serves fs_root as "/").
    #[arg(long, default_value = "/")]
    pub export: String,

    /// Embedding dimension (must match any pre-existing db file).
    #[arg(long, default_value_t = crate::db::DUMMY_DIM)]
    pub dim: usize,
}

/// Extract the port portion of an `ip:port` string (for log lines).
pub fn port_of(addr: &str) -> &str {
    addr.rsplit_once(':').map(|(_, p)| p).unwrap_or("11111")
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
            "--dim", "8",
        ])
        .expect("all flags parse");
        assert_eq!(cli.fs_root, PathBuf::from("/tmp/x"));
        assert_eq!(cli.db_path, ":memory:");
        assert_eq!(cli.addr, "127.0.0.1:11111");
        assert_eq!(cli.export, "/");
        assert_eq!(cli.dim, 8);
    }

    #[test]
    fn cli_defaults_are_sane() {
        let cli = Cli::try_parse_from(["anyhome"]).unwrap();
        assert_eq!(cli.addr, "127.0.0.1:11111");
        assert_eq!(cli.export, "/");
        assert_eq!(cli.dim, crate::db::DUMMY_DIM);
    }

    #[test]
    fn port_of_works() {
        assert_eq!(port_of("1.2.3.4:11111"), "11111");
        assert_eq!(port_of("nope"), "11111");
    }
}
