//! Which key signs, from git's own settings (DESIGN.md section 11.7, decision 32): a
//! dedicated `jtac.signingkey`, else the commit-signing key when git signs with SSH. Pure.

use std::path::PathBuf;

/// What git config says, gathered by the caller with `git config --get`; `None` where unset.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct GitConfig {
    /// `jtac.signingkey`.
    pub jtac_signingkey: Option<String>,
    /// `user.signingkey`.
    pub user_signingkey: Option<String>,
    /// `gpg.format`.
    pub gpg_format: Option<String>,
    /// `gpg.ssh.program`.
    pub gpg_ssh_program: Option<String>,
    /// `user.email`.
    pub user_email: Option<String>,
}

/// A key as git spells `user.signingkey` for SSH.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum KeyRef {
    /// A private key file, or a public key file whose private half is in the agent.
    File(PathBuf),
    /// A public key line, after `key::` or bare from `ssh-`; the private half is in the agent.
    Literal(String),
}

impl KeyRef {
    /// Read git's spelling, including its older bare one.
    pub fn parse(value: &str) -> Self {
        match value.strip_prefix("key::") {
            Some(line) => Self::Literal(line.to_owned()),
            None if value.starts_with("ssh-") => Self::Literal(value.to_owned()),
            None => Self::File(PathBuf::from(value)),
        }
    }
}

/// The signing setup the config resolves to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Resolved {
    /// The key.
    pub key: KeyRef,
    /// The identity the trust file keys on: `user.email`, as git verifies SSH commits.
    pub principal: String,
    /// The ssh-keygen binary: `gpg.ssh.program`, else the name on PATH.
    pub program: PathBuf,
}

/// Why no identity could be resolved. The CLI names `jtac init` for either.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ResolveError {
    /// Neither `jtac.signingkey` nor an SSH `user.signingkey` is set.
    #[error("no signing key: set jtac.signingkey, or user.signingkey with gpg.format=ssh")]
    NoKey,
    /// `user.email` is unset.
    #[error("no principal: user.email is unset")]
    NoPrincipal,
}

/// Decide the identity from the config. An empty value counts as unset.
pub fn resolve(cfg: &GitConfig) -> Result<Resolved, ResolveError> {
    fn set(v: &Option<String>) -> Option<&str> {
        v.as_deref().filter(|s| !s.is_empty())
    }
    let git_ssh = set(&cfg.gpg_format) == Some("ssh");
    let key = set(&cfg.jtac_signingkey)
        .or_else(|| set(&cfg.user_signingkey).filter(|_| git_ssh))
        .ok_or(ResolveError::NoKey)?;
    let principal = set(&cfg.user_email).ok_or(ResolveError::NoPrincipal)?;
    Ok(Resolved {
        key: KeyRef::parse(key),
        principal: principal.to_owned(),
        program: PathBuf::from(set(&cfg.gpg_ssh_program).unwrap_or("ssh-keygen")),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(jtac: &str, user: &str, format: &str, program: &str, email: &str) -> GitConfig {
        let opt = |s: &str| (!s.is_empty()).then(|| s.to_owned());
        GitConfig {
            jtac_signingkey: opt(jtac),
            user_signingkey: opt(user),
            gpg_format: opt(format),
            gpg_ssh_program: opt(program),
            user_email: opt(email),
        }
    }

    #[test]
    fn the_dedicated_key_wins_then_the_git_key_only_when_git_signs_with_ssh() {
        let file = |p: &str| KeyRef::File(PathBuf::from(p));
        let rows = [
            (
                cfg("/k/jtac", "/k/git", "ssh", "", "dev@example.com"),
                Ok(file("/k/jtac")),
            ),
            (
                cfg("", "/k/git", "ssh", "", "dev@example.com"),
                Ok(file("/k/git")),
            ),
            (
                cfg("", "/k/git", "", "", "dev@example.com"),
                Err(ResolveError::NoKey),
            ),
            (
                cfg("", "/k/git", "openpgp", "", "dev@example.com"),
                Err(ResolveError::NoKey),
            ),
            (
                cfg("", "", "ssh", "", "dev@example.com"),
                Err(ResolveError::NoKey),
            ),
            (
                cfg("/k/jtac", "", "", "", ""),
                Err(ResolveError::NoPrincipal),
            ),
            (
                cfg("key::ssh-ed25519 AAAA", "", "", "", "dev@example.com"),
                Ok(KeyRef::Literal("ssh-ed25519 AAAA".into())),
            ),
            (
                cfg("ssh-ed25519 AAAA", "", "", "", "dev@example.com"),
                Ok(KeyRef::Literal("ssh-ed25519 AAAA".into())),
            ),
            (
                cfg(
                    "key::ecdsa-sha2-nistp256 AAAA",
                    "",
                    "",
                    "",
                    "dev@example.com",
                ),
                Ok(KeyRef::Literal("ecdsa-sha2-nistp256 AAAA".into())),
            ),
        ];
        for (c, want) in rows {
            assert_eq!(resolve(&c).map(|r| r.key), want, "{c:?}");
        }
    }

    #[test]
    fn the_program_is_gits_or_the_name_on_path() {
        let on_path = resolve(&cfg("/k", "", "", "", "dev@example.com")).unwrap();
        assert_eq!(on_path.program, PathBuf::from("ssh-keygen"));
        assert_eq!(on_path.principal, "dev@example.com");
        let gits = resolve(&cfg(
            "/k",
            "",
            "",
            "/opt/ssh/bin/ssh-keygen",
            "dev@example.com",
        ))
        .unwrap();
        assert_eq!(gits.program, PathBuf::from("/opt/ssh/bin/ssh-keygen"));
    }
}
