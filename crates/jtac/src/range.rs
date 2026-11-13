//! The commits a range names (DESIGN.md section 11.9): what `rev-list` reads, after
//! `--end-of-options`, so an option never rides in; a rev alone is its whole history, as git
//! reads it. Every list here is one git child, whatever the count.

use std::collections::BTreeSet;
use std::path::Path;

use jtac_checks::git::{GitError, git, git_with};
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

/// One commit of a first-parent walk with every parent it names.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Landed {
    /// The commit.
    pub commit: GitOid,
    /// Its parents in order; the first is the line it landed on.
    pub parents: Vec<GitOid>,
}

/// The first-parent line of `specs`, newest first (DESIGN.md section 11.10). `%P` names every
/// parent whatever `--first-parent` walks; a shallow boundary reads as parentless here, which
/// the caller tells from a root with `has_parent`.
pub fn first_parent_line(
    repo: &Path,
    format: &str,
    command: &str,
    specs: &[String],
) -> Result<Vec<Landed>, Refusal> {
    let mut args = vec![
        "rev-list",
        "--first-parent",
        "--topo-order",
        "--format=%H %P",
        "--end-of-options",
    ];
    args.extend(specs.iter().map(String::as_str));
    let out = git(repo, &args).map_err(|e| Refusal::range(command, specs, &e))?;
    String::from_utf8_lossy(&out)
        .lines()
        .filter(|l| !l.starts_with("commit "))
        .map(|l| {
            let mut ids = l.split_whitespace().map(|hex| oid(format, hex));
            let commit = ids
                .next()
                .ok_or_else(|| Refusal::internal(format!("rev-list printed `{l}`")))??;
            Ok(Landed {
                commit,
                parents: ids.collect::<Result<_, _>>()?,
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

/// What a `Covers::Range` root vouches for: every commit on a path from `base` to `tip`,
/// both included. The writer's exact set is not recoverable from the two ends (DESIGN.md
/// section 9.4); a side line merged in between them is not on such a path and reads absent,
/// never covered. A pair git cannot walk (a planted root naming a blob) vouches for nothing.
pub fn between(
    repo: &Path,
    format: &str,
    base: &GitOid,
    tip: &GitOid,
) -> Result<Vec<GitOid>, Refusal> {
    let span = format!("{}..{}", base.hex(), tip.hex());
    let out = match git(
        repo,
        &["rev-list", "--ancestry-path", "--end-of-options", &span],
    ) {
        Ok(out) => out,
        Err(GitError::Failed { .. }) => return Ok(Vec::new()),
        Err(e) => return Err(Refusal::repository(&RepoError::Git(e))),
    };
    let mut all = String::from_utf8_lossy(&out)
        .lines()
        .map(|hex| oid(format, hex))
        .collect::<Result<Vec<GitOid>, Refusal>>()?;
    all.push(*base);
    Ok(all)
}

/// The object type of each id as git names it, `None` where the clone has no such object.
/// A reader asks before it walks: a blob or tree where a commit should be is a forgery, not
/// missing history, and `merge-base` on one is a fatal error, not an answer.
pub fn kinds(repo: &Path, oids: &[GitOid]) -> Result<Vec<Option<String>>, Refusal> {
    let stdin: String = oids.iter().map(|c| format!("{}\n", c.hex())).collect();
    let out = git_with(
        repo,
        &["cat-file", "--batch-check"],
        Some(stdin.as_bytes()),
        &[],
    )
    .map_err(|e| Refusal::repository(&RepoError::Git(e)))?;
    let text = String::from_utf8_lossy(&out);
    let lines: Vec<&str> = text.lines().collect();
    if lines.len() != oids.len() {
        return Err(Refusal::internal(format!(
            "cat-file --batch-check answered {} lines for {} objects",
            lines.len(),
            oids.len()
        )));
    }
    Ok(lines
        .iter()
        .map(|l| match l.split(' ').collect::<Vec<_>>()[..] {
            [_, kind, _] => Some((*kind).to_owned()),
            _ => None,
        })
        .collect())
}

/// Whether `ancestor` is reachable from `tip`; both must be commits in the clone.
pub fn ancestor(repo: &Path, ancestor: &GitOid, tip: &GitOid) -> Result<bool, Refusal> {
    let (a, t) = (ancestor.hex(), tip.hex());
    match git(repo, &["merge-base", "--is-ancestor", &a, &t]) {
        Ok(_) => Ok(true),
        Err(GitError::Failed {
            status: Some(1), ..
        }) => Ok(false),
        Err(e) => Err(Refusal::repository(&RepoError::Git(e))),
    }
}

/// Whether `commit` has a parent, read from the object itself so a shallow boundary hides
/// nothing.
pub fn has_parent(repo: &Path, commit: &GitOid) -> Result<bool, Refusal> {
    let object = git(repo, &["cat-file", "commit", &commit.hex()])
        .map_err(|e| Refusal::repository(&RepoError::Git(e)))?;
    Ok(String::from_utf8_lossy(&object)
        .lines()
        .take_while(|l| !l.is_empty())
        .any(|l| l.starts_with("parent ")))
}

fn oid(format: &str, hex: &str) -> Result<GitOid, Refusal> {
    repo::oid(format, hex).map_err(Refusal::internal)
}
