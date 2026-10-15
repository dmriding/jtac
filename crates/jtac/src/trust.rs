//! Whom verification trusts (DESIGN.md sections 11.8 and 11.9): `.jtac/signers` in HEAD's
//! tree, read through git so neither a work-tree edit nor a link under `.jtac` (issue 25)
//! can change a verdict, or the file `--signers` names.

use std::path::{Path, PathBuf};

use jtac_sign::TRUST_CAP;

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

    /// For the output: where the keys came from.
    pub fn describe(&self) -> String {
        match self {
            Self::File(path) => path.to_string_lossy().into_owned(),
            Self::Head(_) => format!("HEAD:{TRUST_PATH}"),
            Self::None => format!("none (HEAD has no {TRUST_PATH}; every signer is unknown)"),
        }
    }
}
