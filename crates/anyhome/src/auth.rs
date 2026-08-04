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

#[cfg(test)]
mod tests {
    use super::*;

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
}
