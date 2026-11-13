//! The one policy a `jtac backtest` run judges under (DESIGN.md section 11.10): the `--policy`
//! file, else the newest commit's file read through git, never the work tree. Its hash and
//! where it came from go in the summary, since that choice sets every count.

use std::path::{Path, PathBuf};

use jtac_core::{GitOid, Sha256Digest};
use jtac_policy::{Matchers, POLICY_PATH, Policy};

use super::summary::PolicyFrom;
use crate::cli::BacktestArgs;
use crate::range::Landed;
use crate::refuse::Refusal;
use crate::repo::{self, RepoError};

/// Where the policy was read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Source {
    /// The `--policy` file.
    File(PathBuf),
    /// `.jtac/policy.toml` in the tree of the range's newest commit.
    Newest(GitOid),
}

impl Source {
    /// The source as the summary names it.
    pub fn kind(&self) -> PolicyFrom {
        match self {
            Self::File(_) => PolicyFrom::File,
            Self::Newest(_) => PolicyFrom::NewestCommit,
        }
    }
}

/// The policy, its matchers, and what the summary says of it.
pub struct Chosen {
    /// The parsed policy.
    pub policy: Policy,
    /// Its path and branch matchers.
    pub matchers: Matchers,
    /// Its hash, as a root records it.
    pub hash: Sha256Digest,
    /// Where it was read.
    pub from: PolicyFrom,
}

/// Read and parse the policy for `a`, whose range's newest unit is `newest`.
pub fn choose(
    repo: &Path,
    format: &str,
    a: &BacktestArgs,
    newest: Option<&Landed>,
) -> Result<Chosen, Refusal> {
    let unread =
        |source: &Source, detail: &str| Refusal::backtest_policy_unread(source, detail, &a.range);
    let (source, bytes) = match (&a.policy, newest) {
        (Some(path), _) => {
            let source = Source::File(path.clone());
            let bytes = std::fs::read(path).map_err(|e| unread(&source, &e.to_string()))?;
            (source, bytes)
        }
        (None, Some(unit)) => {
            let git = |e: RepoError| Refusal::backtest_repository(&e, &a.range);
            let source = Source::Newest(unit.commit);
            let head = repo::peel(repo, format, &unit.commit.hex()).map_err(git)?;
            match repo::blob_at(repo, &head, POLICY_PATH) {
                Ok(Some((_, bytes))) => (source, bytes),
                Ok(None) => return Err(Refusal::backtest_no_policy(&a.range)),
                Err(RepoError::NotAFile { .. }) => {
                    return Err(unread(&source, "it is a link or a directory there"));
                }
                Err(e) => return Err(git(e)),
            }
        }
        (None, None) => return Err(Refusal::backtest_no_policy(&a.range)),
    };
    let text = String::from_utf8(bytes).map_err(|_| unread(&source, "it is not UTF-8"))?;
    let (policy, matchers) = jtac_policy::parse(&text)
        .and_then(|p| p.matchers().map(|m| (p, m)))
        .map_err(|e| Refusal::backtest_policy(&source, &e, &a.range))?;
    let hash = jtac_policy::policy_hash(&policy).map_err(Refusal::internal)?;
    Ok(Chosen {
        policy,
        matchers,
        hash,
        from: source.kind(),
    })
}
