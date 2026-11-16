//! The `.jtac/policy.toml` of a tree, as coverage asks of it (DESIGN.md section 15.2,
//! decisions 57 and 60): a root's `unread` and `policy` are its signer's word, so a reader
//! asks the tree. Read from git and never fetched. git answers `missing` both for a path a
//! tree lacks and for an object a partial clone does not hold, so in a partial clone each
//! miss is asked again through the tree: an entry whose blob is not here is `NotHeld`,
//! never "no policy".

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use jtac_checks::git::{GitError, git, git_with};
use jtac_core::GitOid;
use jtac_gate::PolicyKind;
use jtac_policy::POLICY_PATH;

use crate::refuse::Refusal;
use crate::repo::{self, RepoError};

/// What one tree holds at the policy path.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TreePolicy {
    /// No entry.
    None,
    /// A policy this reader reads: its `ci` check ids in policy order, often none.
    Read(Vec<String>),
    /// An entry this reader cannot read: no parse, a version it does not know, not a blob.
    Unreadable,
    /// The id of the object this clone lacks, the blob or the tree that names it.
    NotHeld(String),
}

impl TreePolicy {
    /// What coverage makes of it.
    pub fn kind(&self) -> PolicyKind {
        match self {
            Self::None => PolicyKind::Run,
            Self::Read(ci) if ci.is_empty() => PolicyKind::Run,
            Self::Read(_) | Self::Unreadable => PolicyKind::Ci,
            Self::NotHeld(_) => PolicyKind::NotHeld,
        }
    }

    /// The `ci` checks a root on this tree must list as `unread`; `None` where nothing
    /// shows which.
    pub fn ci_checks(&self) -> Option<&[String]> {
        match self {
            Self::None => Some(&[]),
            Self::Read(ci) => Some(ci),
            Self::Unreadable | Self::NotHeld(_) => None,
        }
    }
}

/// The policy of each of `trees`, each distinct blob parsed once.
pub fn read(
    repo: &Path,
    trees: &BTreeSet<GitOid>,
) -> Result<BTreeMap<GitOid, TreePolicy>, Refusal> {
    let git_err = |e: GitError| Refusal::repository(&RepoError::Git(e));
    if trees.is_empty() {
        return Ok(BTreeMap::new());
    }
    let stdin: String = (trees.iter())
        .map(|t| format!("{}:{POLICY_PATH}\n", t.hex()))
        .collect();
    let args = ["cat-file", "--batch-check"];
    let out = git_with(repo, &args, Some(stdin.as_bytes()), &[]).map_err(git_err)?;
    let text = String::from_utf8_lossy(&out);
    let lines: Vec<&str> = text.lines().collect();
    if lines.len() != trees.len() {
        return Err(Refusal::internal(format!(
            "cat-file --batch-check answered {} lines for {} trees",
            lines.len(),
            trees.len()
        )));
    }
    let partial = promisor(repo)?.is_some();
    let mut entries = Vec::new();
    for (tree, line) in trees.iter().zip(lines) {
        entries.push(match line.split(' ').collect::<Vec<_>>()[..] {
            [blob, "blob", _] => Entry::Blob(blob.to_owned()),
            [_, _, _] => Entry::Is(TreePolicy::Unreadable),
            _ if partial => through_tree(repo, tree)?,
            _ => Entry::Is(TreePolicy::None),
        });
    }
    let blobs: BTreeSet<&str> = (entries.iter())
        .filter_map(|e| match e {
            Entry::Blob(b) => Some(b.as_str()),
            Entry::Is(_) => None,
        })
        .collect();
    let mut parsed = BTreeMap::new();
    for blob in blobs {
        let bytes = git(repo, &["cat-file", "blob", blob]).map_err(git_err)?;
        parsed.insert(blob.to_owned(), parse(&bytes));
    }
    Ok((trees.iter().zip(entries))
        .map(|(tree, entry)| match entry {
            Entry::Blob(blob) => (*tree, parsed[&blob].clone()),
            Entry::Is(policy) => (*tree, policy),
        })
        .collect())
}

/// The remote a partial clone fetches missing objects from, `None` in a full clone. git
/// marks the remote `promisor`; older versions name it in `extensions.partialclone`.
pub fn promisor(repo: &Path) -> Result<Option<String>, Refusal> {
    let git_err = |e: RepoError| Refusal::repository(&e);
    let marked = repo::probe(
        repo,
        &[
            "config",
            "--get-regexp",
            r"^remote\..*\.promisor$",
            "^true$",
        ],
        1,
    )
    .map_err(git_err)?;
    let name = |line: &str| {
        let key = line.split(' ').next()?;
        Some(
            key.strip_prefix("remote.")?
                .strip_suffix(".promisor")?
                .to_owned(),
        )
    };
    match marked.as_deref().and_then(|out| out.lines().find_map(name)) {
        Some(remote) => Ok(Some(remote)),
        None => repo::config(repo, "extensions.partialclone").map_err(git_err),
    }
}

enum Entry {
    Blob(String),
    Is(TreePolicy),
}

/// A miss in a partial clone: the entry as the tree names it, whose object is not here; no
/// entry; or a tree this clone does not hold either.
fn through_tree(repo: &Path, tree: &GitOid) -> Result<Entry, Refusal> {
    let listed = match git(repo, &["ls-tree", &tree.hex(), "--", POLICY_PATH]) {
        Ok(out) => String::from_utf8_lossy(&out).trim().to_owned(),
        Err(GitError::Failed { .. }) => {
            return Ok(Entry::Is(TreePolicy::NotHeld(tree.hex())));
        }
        Err(e) => return Err(Refusal::repository(&RepoError::Git(e))),
    };
    Ok(Entry::Is(
        match listed.split(['\t', ' ']).collect::<Vec<_>>()[..] {
            [""] => TreePolicy::None,
            [_, "blob", blob, ..] => TreePolicy::NotHeld(blob.to_owned()),
            _ => TreePolicy::Unreadable,
        },
    ))
}

fn parse(bytes: &[u8]) -> TreePolicy {
    let policy = std::str::from_utf8(bytes)
        .ok()
        .and_then(|text| jtac_policy::parse(text).ok());
    match policy {
        Some(p) => TreePolicy::Read(p.ci_checks().map(|(id, _)| id.to_owned()).collect()),
        None => TreePolicy::Unreadable,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_policy_is_run_until_it_holds_a_ci_check_or_cannot_be_read() {
        let ci = "version = 2\n[protected]\npaths = [\".github/**\"]\n[checks.test]\nci = { workflow = \".github/workflows/ci.yml\" }\n[checks.fmt]\nrun = [\"true\"]\n";
        let rows = [
            (parse(b"version = 1\n"), PolicyKind::Run, Some(vec![])),
            (
                parse(ci.as_bytes()),
                PolicyKind::Ci,
                Some(vec!["test".to_owned()]),
            ),
            (parse(b"version = 9\n"), PolicyKind::Ci, None),
            (parse(b"\xff"), PolicyKind::Ci, None),
            (TreePolicy::None, PolicyKind::Run, Some(vec![])),
            (TreePolicy::NotHeld("x".into()), PolicyKind::NotHeld, None),
        ];
        for (policy, kind, ci) in rows {
            assert_eq!(policy.kind(), kind, "{policy:?}");
            assert_eq!(policy.ci_checks().map(<[String]>::to_vec), ci, "{policy:?}");
        }
    }
}
