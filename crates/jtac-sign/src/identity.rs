//! The resolved key's fingerprint, asked of `ssh-keygen -l` before anything is signed so
//! the document can name it; `sign` then refuses a signature from any other key.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};

use jtac_core::Signer;

use crate::fingerprint::listed;
use crate::key::{KeyRef, Resolved};
use crate::{SignError, Tool};

/// A resolved key with its fingerprint known.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Identity {
    /// The key ssh-keygen is given.
    pub key: KeyRef,
    /// What a root or lever names as its signer.
    pub signer: Signer,
    /// The ssh-keygen binary.
    pub tool: Tool,
}

/// Ask `ssh-keygen -l` for the fingerprint (DESIGN.md section 11.8). A leading `~` in a path
/// is the home directory, as git reads it.
pub fn identity(r: Resolved) -> Result<Identity, SignError> {
    let home = std::env::home_dir();
    let key = match r.key {
        KeyRef::File(path) => KeyRef::File(expand_home(&path, home.as_deref())?),
        literal @ KeyRef::Literal(_) => literal,
    };
    let tool = Tool::new(expand_home(&r.program, home.as_deref())?);
    let literal = match &key {
        KeyRef::File(_) => None,
        KeyRef::Literal(line) => Some(crate::sign::literal_file(line)?),
    };
    let (path, named): (&Path, PathBuf) = match (&key, &literal) {
        (KeyRef::File(path), _) => (path, path.clone()),
        (_, Some(file)) => (file.as_ref(), PathBuf::from("jtac.signingkey")),
        (KeyRef::Literal(_), None) => unreachable!("a literal key was written to a file"),
    };
    let args: [&OsStr; 3] = ["-l".as_ref(), "-f".as_ref(), path.as_os_str()];
    let out = tool.ok(&args, b"").map_err(|e| match e {
        SignError::Failed { stderr, .. } => SignError::Key {
            path: named.clone(),
            what: stderr,
        },
        other => other,
    })?;
    let fingerprint = listed(&out).ok_or_else(|| SignError::Key {
        path: named,
        what: "ssh-keygen -l printed no fingerprint".into(),
    })?;
    Ok(Identity {
        key,
        signer: Signer {
            principal: r.principal,
            fingerprint,
        },
        tool,
    })
}

/// The ssh-keygen a verifier runs: `gpg.ssh.program` as [`resolve`](crate::resolve) reads it,
/// `~` expanded. Checking a signature needs no key and no principal.
pub fn verifier(gpg_ssh_program: Option<&str>) -> Result<Tool, SignError> {
    let program = crate::key::program(gpg_ssh_program);
    Ok(Tool::new(expand_home(
        &program,
        std::env::home_dir().as_deref(),
    )?))
}

/// `~` and `~/...` only. Git also expands `~user` through the password database; jtac has no
/// safe way to, so it refuses rather than sign with a different key than git would.
fn expand_home(path: &Path, home: Option<&Path>) -> Result<PathBuf, SignError> {
    let mut parts = path.components();
    let first = match parts.next() {
        Some(std::path::Component::Normal(first)) => first.to_string_lossy(),
        _ => return Ok(path.to_path_buf()),
    };
    if first == "~" {
        return Ok(home.map_or_else(|| path.to_path_buf(), |h| h.join(parts.as_path())));
    }
    if first.starts_with('~') {
        return Err(SignError::Key {
            path: path.to_path_buf(),
            what: "`~user` is not expanded; write the full path".into(),
        });
    }
    Ok(path.to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_leading_tilde_is_the_home_directory_and_a_named_user_is_refused() {
        let home = Path::new("/home/dev");
        let rows = [
            ("~", "/home/dev"),
            ("~/.ssh/key", "/home/dev/.ssh/key"),
            ("/abs/~/key", "/abs/~/key"),
            ("rel/key", "rel/key"),
            ("ssh-keygen", "ssh-keygen"),
        ];
        for (given, want) in rows {
            let got = expand_home(Path::new(given), Some(home)).unwrap();
            assert_eq!(got, PathBuf::from(want));
            let no_home = expand_home(Path::new(given), None).unwrap();
            assert_eq!(no_home, PathBuf::from(given));
        }
        let err = expand_home(Path::new("~dev/.ssh/key"), Some(home)).unwrap_err();
        assert!(matches!(err, SignError::Key { .. }), "{err}");
    }

    #[test]
    fn the_verifier_is_gits_program_or_ssh_keygen_on_path() {
        let rows = [
            (None, "ssh-keygen"),
            (Some(""), "ssh-keygen"),
            (Some("/opt/ssh/bin/ssh-keygen"), "/opt/ssh/bin/ssh-keygen"),
        ];
        for (given, want) in rows {
            assert_eq!(
                verifier(given).unwrap().program(),
                Path::new(want),
                "{given:?}"
            );
        }
        assert!(verifier(Some("~dev/ssh-keygen")).is_err());
    }
}
