//! The signing key `jtac init` finds or makes (DESIGN.md sections 11.7 and 11.8, decision 34):
//! the key git config names, else a dedicated ed25519 key under the user config directory,
//! named by the global `jtac.signingkey` so one person has one identity across repositories.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use jtac_checks::git::git;
use jtac_sign::{Identity, KeyRef, ResolveError, SignError};

use crate::identity::{git_config, resolved};
use crate::init::{ask, slashes};
use crate::refuse::Refusal;

/// What init does about the key.
#[derive(Debug)]
pub enum Key {
    /// git config already names a working key; `from` is the setting that named it.
    Kept {
        /// The key and its fingerprint.
        id: Identity,
        /// `jtac.signingkey`, or `user.signingkey` when git signs with SSH.
        from: &'static str,
    },
    /// No key is named: make one at `path` (or reuse the file there) and name it globally.
    New {
        /// `gpg.ssh.program`, else `ssh-keygen`.
        program: PathBuf,
        /// `user.email`, the principal and the key comment.
        email: String,
        /// `<user config dir>/jtac/id_ed25519`.
        path: PathBuf,
        /// The file is already there, from an earlier init whose config was since removed.
        exists: bool,
    },
}

/// The user config directory: `XDG_CONFIG_HOME` anywhere, else the platform's own. A relative
/// or empty value counts as unset, as the XDG spec says.
pub fn config_dir(os: &str, var: impl Fn(&str) -> Option<OsString>) -> Option<PathBuf> {
    let abs = |name: &str| var(name).map(PathBuf::from).filter(|p| p.is_absolute());
    abs("XDG_CONFIG_HOME").or_else(|| match os {
        "macos" => abs("HOME").map(|h| h.join("Library").join("Application Support")),
        "windows" => abs("APPDATA"),
        _ => abs("HOME").map(|h| h.join(".config")),
    })
}

/// Decide the key step from git config and the environment; writes nothing. A named key is
/// probed now, so a broken one refuses before anything is written.
pub fn plan(repo: &Path) -> Result<Key, Refusal> {
    let cfg = git_config(repo)?;
    let set = |v: &Option<String>| v.clone().filter(|s| !s.is_empty());
    let email = set(&cfg.user_email).ok_or_else(Refusal::init_no_email)?;
    match resolved(repo, &cfg) {
        Ok(r) => Ok(Key::Kept {
            id: jtac_sign::identity(r).map_err(|e| Refusal::init_key(&e))?,
            from: if set(&cfg.jtac_signingkey).is_some() {
                "jtac.signingkey"
            } else {
                "user.signingkey with gpg.format ssh"
            },
        }),
        Err(ResolveError::NoKey) => {
            let dir = config_dir(std::env::consts::OS, |k| std::env::var_os(k))
                .ok_or_else(Refusal::init_no_config_dir)?;
            let path = dir.join("jtac").join("id_ed25519");
            Ok(Key::New {
                program: PathBuf::from(set(&cfg.gpg_ssh_program).unwrap_or("ssh-keygen".into())),
                email,
                exists: path.symlink_metadata().is_ok(),
                path,
            })
        }
        Err(e) => Err(Refusal::no_identity(&e)),
    }
}

/// Make the key unless it exists, name it in the global config, then resolve it again and ask
/// ssh-keygen for its fingerprint, so a key that cannot sign refuses here and not at push.
pub fn create(
    repo: &Path,
    program: &Path,
    email: &str,
    path: &Path,
    exists: bool,
) -> Result<Identity, Refusal> {
    if !exists {
        generate(program, email, path)?;
    }
    let value = slashes(path);
    let cfg = jtac_sign::GitConfig {
        jtac_signingkey: Some(value.clone()),
        ..git_config(repo)?
    };
    let r = resolved(repo, &cfg).map_err(|e| Refusal::no_identity(&e))?;
    let id = jtac_sign::identity(r).map_err(|e| Refusal::init_key_file(path, &e))?;
    git(repo, &["config", "--global", "jtac.signingkey", &value])
        .map_err(|e| Refusal::init_config_write(&value, &e))?;
    Ok(id)
}

/// `ssh-keygen` with no passphrase and stdin closed, into a directory only the user reads.
fn generate(program: &Path, email: &str, path: &Path) -> Result<(), Refusal> {
    let dir = path.parent().expect("the key path has a parent");
    private_dir(dir).map_err(|e| Refusal::init_io("the key directory", dir, &e))?;
    let comment = format!("jtac {email}");
    let out = Command::new(program)
        .args(["-q", "-t", "ed25519", "-N", "", "-C", &comment, "-f"])
        .arg(path)
        .stdin(Stdio::null())
        .output();
    match out {
        Ok(o) if o.status.success() => Ok(()),
        Ok(o) => {
            let stderr = String::from_utf8_lossy(&o.stderr);
            let what = stderr.lines().find(|l| !l.trim().is_empty());
            let what = what.map_or_else(|| o.status.to_string(), str::to_owned);
            Err(Refusal::init_keygen(program, path, &what))
        }
        Err(e) => Err(Refusal::init_keygen(program, path, &e.to_string())),
    }
}

#[cfg(unix)]
fn private_dir(dir: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::DirBuilderExt;
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(dir)
}

#[cfg(not(unix))]
fn private_dir(dir: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)
}

/// The allowed-signers line for `id`, when its public half reads without the private key:
/// a key line, a `.pub` file, or the `.pub` beside a private key file.
pub fn signers_line(id: &Identity) -> Option<String> {
    let public = match &id.key {
        KeyRef::Literal(line) => line.clone(),
        KeyRef::File(path) => {
            let mut public = path.as_os_str().to_owned();
            if path.extension().is_none_or(|e| e != "pub") {
                public.push(".pub");
            }
            std::fs::read_to_string(public).ok()?.trim().to_owned()
        }
    };
    Some(format!("{} {public}", id.signer.principal))
}

impl Refusal {
    /// `user.email` is unset, so there is no principal.
    pub fn init_no_email() -> Self {
        ask(
            "`user.email` is not set in git config.",
            "The email is the principal every root and lever names, the identity git itself \
             verifies SSH commits with; a key with no principal signs evidence nobody can \
             attribute.",
            "Run `git config --global user.email \"<you@example.com>\"` with your address, \
             then `jtac init` again.",
        )
    }

    /// No variable names a user config directory to keep a generated key in.
    pub fn init_no_config_dir() -> Self {
        ask(
            "no user config directory: `XDG_CONFIG_HOME`, `HOME`, and `APPDATA` are unset or \
             not absolute.",
            "init keeps the key it generates under the user config directory, outside every \
             repository, so it never lands in a commit.",
            "Run `export XDG_CONFIG_HOME=\"$HOME/.config\"` with HOME set (or name a key you \
             have: `git config --global jtac.signingkey <path to an SSH private key>`), then \
             `jtac init` again.",
        )
    }

    /// ssh-keygen did not make the key.
    pub fn init_keygen(program: &Path, path: &Path, what: &str) -> Self {
        ask(
            format!(
                "`{}` could not generate `{}`: {what}.",
                program.to_string_lossy(),
                path.to_string_lossy()
            ),
            "Roots are signed with SSHSIG through ssh-keygen; with no key, `jtac check` has \
             nothing to sign with and writes no root.",
            format!(
                "Run `ssh-keygen -t ed25519 -N \"\" -f '{}'` yourself (install OpenSSH if it \
                 is missing), then `jtac init` again.",
                path.to_string_lossy()
            ),
        )
    }

    /// The key git config names cannot be read by ssh-keygen.
    pub fn init_key(err: &SignError) -> Self {
        ask(
            format!("the signing key does not work: {err}."),
            "init proves the key now, so a key that cannot sign is found here and not when \
             `jtac check` has run every check and has nothing to sign with.",
            "Fix the key the config names (`git config --get jtac.signingkey`, or `git config \
             --get user.signingkey`), or unset it to let init generate one: `git config \
             --global --unset jtac.signingkey`; then `jtac init` again.",
        )
    }

    /// The file at the default key path cannot be read by ssh-keygen; nothing was named.
    pub fn init_key_file(path: &Path, err: &SignError) -> Self {
        let path = slashes(path);
        ask(
            format!("the key file `{path}` does not work: {err}."),
            "init names a key in the global config only after ssh-keygen has read it, so a \
             broken file is never the key every repository signs with.",
            format!(
                "Move the file aside, `mv '{path}' '{path}.bad'`, so init generates a fresh \
                 key there; or name a key you have: `git config --global jtac.signingkey <path \
                 to an SSH private key>`; then `jtac init` again."
            ),
        )
    }

    /// `git config --global` refused the write.
    pub fn init_config_write(value: &str, err: &jtac_checks::git::GitError) -> Self {
        ask(
            format!("could not set `jtac.signingkey` in the global git config: {err}."),
            "The key is named in the global config so every repository signs as the same \
             person; a key nobody names is a key `jtac check` never finds.",
            format!("Run `git config --global jtac.signingkey '{value}'`, then `jtac init` again."),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    type Row<'a> = (&'a str, &'a [(&'a str, &'a OsString)], Option<PathBuf>);

    #[test]
    fn xdg_wins_then_each_platform_has_its_own_directory_and_relative_is_unset() {
        let root = if cfg!(windows) { "C:\\w" } else { "/w" };
        let at = |p: &str| PathBuf::from(root).join(p);
        let xdg = at("xdg").into_os_string();
        let home = at("home").into_os_string();
        let app = at("app").into_os_string();
        let (rel, empty) = (OsString::from("rel"), OsString::new());
        let rows: [Row; 8] = [
            (
                "linux",
                &[("XDG_CONFIG_HOME", &xdg), ("HOME", &home)],
                Some(at("xdg")),
            ),
            (
                "linux",
                &[("HOME", &home)],
                Some(at("home").join(".config")),
            ),
            (
                "macos",
                &[("HOME", &home)],
                Some(at("home").join("Library").join("Application Support")),
            ),
            (
                "macos",
                &[("XDG_CONFIG_HOME", &xdg), ("HOME", &home)],
                Some(at("xdg")),
            ),
            (
                "windows",
                &[("APPDATA", &app), ("HOME", &home)],
                Some(at("app")),
            ),
            ("windows", &[("HOME", &home)], None),
            (
                "linux",
                &[("XDG_CONFIG_HOME", &rel), ("HOME", &home)],
                Some(at("home").join(".config")),
            ),
            ("linux", &[("XDG_CONFIG_HOME", &empty)], None),
        ];
        for (os, vars, want) in rows {
            let var = |k: &str| {
                vars.iter()
                    .find(|(n, _)| *n == k)
                    .map(|(_, v)| (*v).clone())
            };
            assert_eq!(config_dir(os, var), want, "{os} {vars:?}");
        }
    }
}
