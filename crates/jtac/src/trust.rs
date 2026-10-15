//! Whom verification trusts (DESIGN.md sections 11.8 and 11.9): `.jtac/signers` in HEAD's
//! tree, read through git so neither a work-tree edit nor a link under `.jtac` (issue 25)
//! can change a verdict, or the file `--signers` names.

use std::path::{Path, PathBuf};

use jtac_sign::{SignError, TRUST_CAP};

use crate::refuse::Refusal;
use crate::repo::{Head, RepoError, blob_at};

/// Where the allowed-signers lines come from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Signers {
    /// `--signers`.
    File(PathBuf),
    /// `.jtac/signers` at HEAD.
    Head(Vec<u8>),
    /// HEAD has no `.jtac/signers`.
    None,
}

/// The trust file at `HEAD`.
pub const TRUST_PATH: &str = ".jtac/signers";

impl Signers {
    /// `--signers` when given, else HEAD's file.
    pub fn gather(repo: &Path, head: &Head, signers: Option<&Path>) -> Result<Self, Refusal> {
        if let Some(path) = signers {
            return Ok(Self::File(path.to_path_buf()));
        }
        match blob_at(repo, head, TRUST_PATH) {
            Ok(Some((_, bytes))) if bytes.len() as u64 > TRUST_CAP => {
                Err(Refusal::trust("larger than the trust file cap"))
            }
            Ok(Some((_, bytes))) => Ok(Self::Head(bytes)),
            Ok(None) => Ok(Self::None),
            Err(RepoError::NotAFile { .. }) => Err(Refusal::trust("not a regular file")),
            Err(e) => Err(Refusal::repository(&e)),
        }
    }

    /// As the signature verifier takes it.
    pub fn as_sign(&self) -> jtac_sign::Trust<'_> {
        match self {
            Self::File(path) => jtac_sign::Trust::File(path),
            Self::Head(bytes) => jtac_sign::Trust::Bytes(bytes),
            Self::None => jtac_sign::Trust::None,
        }
    }

    /// A verifier error as a refusal: a `--signers` file that could not be read is about
    /// the file; an `Io` elsewhere (the verifier's temp dir) and all else is about ssh-keygen
    /// (`a_signers_refusal_is_about_the_file_only_when_the_file_failed`).
    pub fn refusal(&self, command: &str, err: &SignError) -> Refusal {
        match (self, err) {
            (Self::File(file), SignError::Trust { .. }) => {
                Refusal::signers_file(command, file, err)
            }
            (Self::File(file), SignError::Io { path, .. }) if path == file => {
                Refusal::signers_file(command, file, err)
            }
            _ => Refusal::verifier(err),
        }
    }

    /// For the output: where the keys came from.
    pub fn describe(&self) -> String {
        match self {
            Self::File(path) => path.to_string_lossy().into_owned(),
            Self::Head(_) => format!("HEAD:{TRUST_PATH}"),
            Self::None => format!("none (HEAD has no {TRUST_PATH}; every signer is unknown)"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_signers_refusal_is_about_the_file_only_when_the_file_failed() {
        let file = PathBuf::from("/workspace/s");
        let io = |path: &str| SignError::Io {
            path: path.into(),
            source: std::io::Error::from(std::io::ErrorKind::PermissionDenied),
        };
        let signers = Signers::File(file.clone());
        let own = signers.refusal("replay", &io("/workspace/s"));
        assert!(
            own.how.contains("`jtac replay --signers /workspace/s`"),
            "{own:?}"
        );
        let temp = signers.refusal("replay", &io("/workspace/tmp"));
        assert!(temp.how.contains("ssh-keygen"), "{temp:?}");
        let head = Signers::Head(Vec::new()).refusal("verify", &io("/workspace/s"));
        assert!(head.how.contains("ssh-keygen"), "{head:?}");
    }
}
