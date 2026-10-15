//! The commits a range names (DESIGN.md section 11.9): what `rev-list` reads, after
//! `--end-of-options`, so an option never rides in; a rev alone is its whole history, as git
//! reads it. Every list here is one git child, whatever the count.

use std::collections::BTreeSet;
use std::path::Path;

use jtac_checks::git::{git, git_with};
use jtac_core::GitOid;

use crate::refuse::Refusal;
use crate::repo::{self, RepoError};

/// One commit of the range.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Listed {
    /// The commit.
    pub commit: GitOid,
    /// Its tree.
    pub tree: GitOid,
}

/// The commits `specs` names, topologically, newest first.
pub fn commits(
    repo: &Path,
    format: &str,
    command: &str,
    specs: &[String],
) -> Result<Vec<Listed>, Refusal> {
    let mut args = vec![
        "rev-list",
        "--topo-order",
        "--format=%H %T",
        "--end-of-options",
    ];
    args.extend(specs.iter().map(String::as_str));
    let out = git(repo, &args).map_err(|e| Refusal::range(command, specs, &e))?;
    String::from_utf8_lossy(&out)
        .lines()
        .filter(|l| !l.starts_with("commit "))
        .map(|l| {
            let (c, t) = l
                .split_once(' ')
                .ok_or_else(|| Refusal::internal(format!("rev-list printed `{l}`")))?;
            Ok(Listed {
                commit: oid(format, c)?,
                tree: oid(format, t)?,
            })
        })
        .collect()
}

/// The tree of each commit, `None` where the clone has no such commit.
pub fn trees(
    repo: &Path,
    format: &str,
    commits: &[GitOid],
) -> Result<Vec<Option<GitOid>>, Refusal> {
    if commits.is_empty() {
        return Ok(Vec::new());
    }
    let stdin: String = commits
        .iter()
        .map(|c| format!("{}^{{tree}}\n", c.hex()))
        .collect();
    let out = git_with(
        repo,
        &["cat-file", "--batch-check"],
        Some(stdin.as_bytes()),
        &[],
    )
    .map_err(|e| Refusal::repository(&RepoError::Git(e)))?;
    let text = String::from_utf8_lossy(&out);
    let lines: Vec<&str> = text.lines().collect();
    if lines.len() != commits.len() {
        return Err(Refusal::internal(format!(
            "cat-file --batch-check answered {} lines for {} commits",
            lines.len(),
            commits.len()
        )));
    }
    lines
        .iter()
        .map(|l| match l.split(' ').collect::<Vec<_>>()[..] {
            [hex, "tree", _] => oid(format, hex).map(Some),
            _ => Ok(None),
        })
        .collect()
}

/// Commits reachable from `tip` and from none of `exclude`; an excluded commit the clone
/// lacks is ignored, which can only widen the set.
pub fn reachable(
    repo: &Path,
    format: &str,
    tip: &GitOid,
    exclude: &[GitOid],
) -> Result<BTreeSet<GitOid>, Refusal> {
    let mut stdin = tip.hex();
    stdin.push('\n');
    for c in exclude {
        stdin.push('^');
        stdin.push_str(&c.hex());
        stdin.push('\n');
    }
    let out = git_with(
        repo,
        &["rev-list", "--ignore-missing", "--stdin"],
        Some(stdin.as_bytes()),
        &[],
    )
    .map_err(|e| Refusal::repository(&RepoError::Git(e)))?;
    String::from_utf8_lossy(&out)
        .lines()
        .map(|hex| oid(format, hex))
        .collect()
}

fn oid(format: &str, hex: &str) -> Result<GitOid, Refusal> {
    repo::oid(format, hex).map_err(Refusal::internal)
}
