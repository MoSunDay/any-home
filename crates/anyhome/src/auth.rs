//! Agent identity & authorization for the magic-path namespace.
//!
//! `init-pro-4` delivered *logical* namespace isolation (`/agents/<id>/...`)
//! but never verified *who* the caller is — any client could impersonate any
//! agent. This module is the authorization half: given the caller's AUTH_SYS
//! `uid` (surfaced from the RPC layer via [`crate::dispatcher`] →
//! `nfsserve::identity`) and the target `agent_id`, decide whether the call is
//! permitted.
//!
//! ## Threat model — *soft* auth
//! NFSv3 AUTH_SYS is **not** cryptographically secure: `uid`/`gid` are
//! client-asserted and trivially spoofable. This defends against **accidental
//! cross-agent contamination** (a misconfigured agent writing to a neighbour's
//! namespace) and casual probing — *not* against a determined adversary. For
//! hard security, deploy in front of an mTLS-terminated network or upgrade to
//! RPCSEC_GSS/Kerberos. See README "已知限制".
//!
//! ## Backward compatibility
//! When no credentials are configured the policy is [`AuthPolicy::Open`]:
//! every call is permitted, identical to `init-pro-4`. Existing tests and
//! deployments are untouched.

use std::collections::{BTreeSet, HashMap};
use std::path::Path;

use crate::cli::Cli;

/// Caller credential derived from the NFS RPC AUTH_SYS flavor.
///
/// `uid == 0` denotes an *anonymous* caller — either no AUTH_SYS credential was
/// supplied (AUTH_NULL) or the code is running outside the server (unit tests).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CallerCred {
    pub uid: u32,
}

impl CallerCred {
    /// `true` when the caller supplied no AUTH_SYS credential.
    pub fn is_anonymous(&self) -> bool {
        self.uid == 0
    }
}

/// Authorization policy for agent namespaces.
///
/// Construct with [`AuthPolicy::from_creds`]; when the credential list is empty
/// this collapses to [`AuthPolicy::Open`] (auth disabled, `init-pro-4` behavior).
#[derive(Clone, Debug, Default)]
pub enum AuthPolicy {
    /// No credentials configured — every caller may act as any agent.
    #[default]
    Open,
    /// Credentials configured. A caller's `uid` must be registered for an
    /// `agent_id` to access `/agents/<id>/...`. The default agent (`""`) stays
    /// public; anonymous callers are confined to it.
    Enforced {
        allow: HashMap<u32, BTreeSet<String>>,
    },
}

impl AuthPolicy {
    /// `true` when authorization checks are active (non-default credentials set).
    pub fn is_enforced(&self) -> bool {
        matches!(self, AuthPolicy::Enforced { .. })
    }

    /// Returns `true` if `caller` is permitted to act as `agent_id`.
    ///
    /// Rules (only meaningful under [`AuthPolicy::Enforced`]):
    /// - the default agent (`agent_id == ""`) is **always** public;
    /// - a named agent requires the caller's `uid` to be registered for it;
    /// - anonymous callers (`uid == 0`) can only reach the default agent.
    pub fn permits(&self, caller: CallerCred, agent_id: &str) -> bool {
        match self {
            AuthPolicy::Open => true,
            AuthPolicy::Enforced { allow } => {
                if agent_id.is_empty() {
                    return true;
                }
                allow
                    .get(&caller.uid)
                    .map(|ids| ids.contains(agent_id))
                    .unwrap_or(false)
            }
        }
    }

    /// Build a policy from a credential list. An empty list yields [`Open`]
    /// (auth disabled), preserving backward compatibility.
    pub fn from_creds(creds: &[AgentCred]) -> Self {
        if creds.is_empty() {
            return AuthPolicy::Open;
        }
        let mut allow: HashMap<u32, BTreeSet<String>> = HashMap::new();
        for c in creds {
            allow.entry(c.uid).or_default().insert(c.agent_id.clone());
        }
        AuthPolicy::Enforced { allow }
    }
}

/// A single agent credential: `agent_id` owned by AUTH_SYS `uid`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AgentCred {
    pub agent_id: String,
    pub uid: u32,
}

impl AgentCred {
    pub fn new(agent_id: impl Into<String>, uid: u32) -> Self {
        AgentCred {
            agent_id: agent_id.into(),
            uid,
        }
    }

    /// clap `value_parser`: parses `"<agent_id>:<uid>"` into an [`AgentCred`].
    pub fn parse_clap(
        input: &str,
    ) -> Result<Self, Box<dyn std::error::Error + Send + Sync + 'static>> {
        input.parse().map_err(Into::into)
    }
}

impl std::str::FromStr for AgentCred {
    type Err = String;

    /// Parse `"<agent_id>:<uid>"`, e.g. `"botA:1001"`.
    /// `agent_id` may be empty (registers `uid` as owning the default agent,
    /// which is a no-op for enforcement since the default agent is always
    /// public) but is still accepted for config symmetry.
    /// `uid` must be non-zero: uid 0 denotes anonymous callers under AUTH_SYS.
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let (agent_id, uid_str) = s
            .rsplit_once(':')
            .ok_or_else(|| format!("expected \"<agent_id>:<uid>\", got {:?}", s))?;
        let uid = uid_str
            .trim()
            .parse::<u32>()
            .map_err(|e| format!("invalid uid {:?}: {}", uid_str.trim(), e))?;
        if uid == 0 {
            return Err(
                "uid 0 is reserved for anonymous callers and cannot be bound to an agent: \
                 under AUTH_SYS, real root (uid 0) is indistinguishable from AUTH_NULL, \
                 so binding uid 0 would grant anonymous callers that agent's access"
                    .to_string(),
            );
        }
        Ok(AgentCred {
            agent_id: agent_id.trim().to_string(),
            uid,
        })
    }
}

/// Parse a credentials file: one `"<agent_id>:<uid>"` per line. Blank lines and
/// `#` comments are ignored. Whitespace around each token is trimmed.
pub fn parse_creds_file(path: &Path) -> Result<Vec<AgentCred>, String> {
    let raw = std::fs::read_to_string(path)
        .map_err(|e| format!("cannot read creds file {:?}: {}", path, e))?;
    parse_creds_text(&raw)
}

/// Parse credentials from a text buffer (one entry per non-comment line).
pub fn parse_creds_text(raw: &str) -> Result<Vec<AgentCred>, String> {
    let mut out = Vec::new();
    for (lineno, line) in raw.lines().enumerate() {
        // Drop inline comments (`# ...`), then trim. Blank lines are skipped.
        let line = line.split('#').next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        let cred = line
            .parse::<AgentCred>()
            .map_err(|e| format!("line {}: {}", lineno + 1, e))?;
        out.push(cred);
    }
    Ok(out)
}

/// CLI-policy assembly half: build the [`AuthPolicy`] from the CLI credential
/// sources.
///
/// Merges repeated `--agent-uid <id:uid>` flags with the entries of an
/// optional `--agent-creds <file>` (flags ∪ file). An empty credential set
/// yields [`AuthPolicy::Open`] (auth disabled — backward compatible).
/// Duplicate `(agent_id, uid)` bindings are de-duplicated by the policy's set
/// semantics. A missing or malformed creds file is an error, wrapped with the
/// offending path for operator-friendly diagnostics.
pub fn build_auth_policy(cli: &Cli) -> anyhow::Result<AuthPolicy> {
    let mut creds = cli.agent_uid.clone();
    if let Some(path) = &cli.agent_creds {
        let file_creds =
            parse_creds_file(path).map_err(|e| anyhow::anyhow!("--agent-creds {path:?}: {e}"))?;
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

#[cfg(test)]
mod tests {
    use super::*;

    use clap::Parser;

    fn cred(uid: u32) -> CallerCred {
        CallerCred { uid }
    }

    #[test]
    fn open_policy_allows_everyone() {
        let p = AuthPolicy::Open;
        assert!(p.permits(cred(0), "anyone"));
        assert!(p.permits(cred(9999), "botB"));
        assert!(!p.is_enforced());
    }

    #[test]
    fn empty_creds_collapse_to_open() {
        let p = AuthPolicy::from_creds(&[]);
        assert!(matches!(p, AuthPolicy::Open));
    }

    #[test]
    fn enforced_permits_registered_uid_only() {
        let p = AuthPolicy::from_creds(&[
            AgentCred::new("botA", 1001),
            AgentCred::new("botB", 1002),
        ]);
        assert!(p.is_enforced());
        assert!(p.permits(cred(1001), "botA"));
        assert!(!p.permits(cred(1001), "botB")); // impersonation blocked
        assert!(p.permits(cred(1002), "botB"));
    }

    #[test]
    fn default_agent_always_public() {
        let p = AuthPolicy::from_creds(&[AgentCred::new("botA", 1001)]);
        assert!(p.permits(cred(0), "")); // anonymous
        assert!(p.permits(cred(9999), "")); // unknown uid
    }

    #[test]
    fn anonymous_confined_to_default() {
        let p = AuthPolicy::from_creds(&[AgentCred::new("botA", 1001)]);
        assert!(!p.permits(cred(0), "botA"));
        assert!(!p.permits(cred(0), "botB"));
    }

    #[test]
    fn one_uid_many_agents() {
        let p = AuthPolicy::from_creds(&[
            AgentCred::new("botA", 1001),
            AgentCred::new("botB", 1001),
        ]);
        assert!(p.permits(cred(1001), "botA"));
        assert!(p.permits(cred(1001), "botB"));
        assert!(!p.permits(cred(1002), "botA"));
    }

    #[test]
    fn parse_agent_cred() {
        let c: AgentCred = "botA:1001".parse().unwrap();
        assert_eq!(c, AgentCred::new("botA", 1001));
        assert!("botA".parse::<AgentCred>().is_err());
        assert!("botA:notanumber".parse::<AgentCred>().is_err());
    }

    #[test]
    fn parse_rejects_uid_zero() {
        // uid 0 means anonymous (AUTH_NULL) and cannot be safely bound to an
        // agent, since real root and anonymous callers are indistinguishable.
        let err = "botA:0".parse::<AgentCred>().unwrap_err();
        assert!(
            err.contains("uid 0"),
            "expected error to mention uid 0, got: {err}"
        );
    }

    #[test]
    fn parse_creds_text_ignores_comments() {
        let creds = parse_creds_text(
            "# header\n\
             botA:1001   # owner of namespace A\n\
             \n\
              botB : 1002  \n",
        )
        .unwrap();
        assert_eq!(creds.len(), 2);
        assert_eq!(creds[0], AgentCred::new("botA", 1001));
        assert_eq!(creds[1], AgentCred::new("botB", 1002));
    }

    #[test]
    fn parse_creds_text_rejects_bad_uid() {
        assert!(parse_creds_text("botA:notanumber").is_err());
        assert!(parse_creds_text("no_colon_here").is_err());
    }

    #[test]
    fn is_anonymous_flag() {
        assert!(cred(0).is_anonymous());
        assert!(!cred(1).is_anonymous());
    }

    // ---- build_auth_policy (CLI assembly half) ------------------------------

    /// Helper: parse CLI args exactly as the binary would (all other args have
    /// defaults, so only the auth-related flags need to be passed).
    fn cli_from(args: &[&str]) -> Cli {
        Cli::try_parse_from(args).expect("valid CLI args")
    }

    #[test]
    fn build_policy_from_flags_only_is_enforced() {
        let cli = cli_from(&["anyhome", "--agent-uid", "botA:1001"]);
        let p = build_auth_policy(&cli).unwrap();
        assert!(p.is_enforced());
        assert!(p.permits(cred(1001), "botA"));
        assert!(!p.permits(cred(1002), "botA")); // wrong uid denied
        assert!(!p.permits(cred(1001), "botB")); // wrong agent denied
    }

    #[test]
    fn build_policy_from_creds_file_only() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("creds.txt");
        std::fs::write(&path, "botA:1001\nbotB:1002\n").unwrap();
        let cli = cli_from(&["anyhome", "--agent-creds", path.to_str().unwrap()]);
        let p = build_auth_policy(&cli).unwrap();
        assert!(p.is_enforced());
        assert!(p.permits(cred(1001), "botA"));
        assert!(p.permits(cred(1002), "botB"));
        assert!(!p.permits(cred(1001), "botB")); // impersonation blocked
    }

    #[test]
    fn build_policy_dedups_identical_flag_and_file_binding() {
        // The same binding from both sources must merge cleanly (BTreeSet
        // dedup) — no error, still Enforced.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("creds.txt");
        std::fs::write(&path, "botA:1001\n").unwrap();
        let cli = cli_from(&[
            "anyhome",
            "--agent-uid",
            "botA:1001",
            "--agent-creds",
            path.to_str().unwrap(),
        ]);
        let p = build_auth_policy(&cli).unwrap();
        assert!(p.is_enforced());
        assert!(p.permits(cred(1001), "botA"));
    }

    #[test]
    fn build_policy_unions_flag_and_file_bindings() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("creds.txt");
        std::fs::write(&path, "botB:1002\n").unwrap();
        let cli = cli_from(&[
            "anyhome",
            "--agent-uid",
            "botA:1001",
            "--agent-creds",
            path.to_str().unwrap(),
        ]);
        let p = build_auth_policy(&cli).unwrap();
        assert!(p.is_enforced());
        assert!(p.permits(cred(1001), "botA")); // from the flag
        assert!(p.permits(cred(1002), "botB")); // from the file
        assert!(!p.permits(cred(9999), "botA")); // unregistered uid denied
        assert!(!p.permits(cred(9999), "botB"));
    }

    #[test]
    fn build_policy_no_sources_is_open() {
        let cli = cli_from(&["anyhome"]);
        let p = build_auth_policy(&cli).unwrap();
        assert!(!p.is_enforced());
        assert!(p.permits(cred(0), "anything")); // Open permits everyone
        assert!(p.permits(cred(9999), "botZ"));
    }

    #[test]
    fn build_policy_missing_creds_file_names_path_in_error() {
        let missing = "/definitely/not/here/creds.txt";
        let cli = cli_from(&["anyhome", "--agent-creds", missing]);
        let err = build_auth_policy(&cli).unwrap_err().to_string();
        assert!(
            err.contains(missing),
            "error should name the missing path, got: {err}"
        );
    }

    #[test]
    fn build_policy_malformed_creds_file_names_path_in_error() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("creds.txt");
        // uid 0 is rejected by parse_creds_text, so the file is malformed.
        std::fs::write(&path, "botA:0\n").unwrap();
        let path_str = path.to_str().unwrap().to_string();
        let cli = cli_from(&["anyhome", "--agent-creds", &path_str]);
        let err = build_auth_policy(&cli).unwrap_err().to_string();
        assert!(
            err.contains(&path_str),
            "error should name the malformed path, got: {err}"
        );
    }
}
