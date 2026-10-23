//! Where a root sits in history (DESIGN.md section 9.4, decision 33): the commits it covers
//! and the roots it links to, found by commit alone. A root for an identical tree is a
//! reader's rule (`audit`, `pre-push`), never a writer's.

use std::collections::BTreeSet;
use std::path::Path;

use jtac_checks::git::{git, git_with};
use jtac_core::{Covers, GitOid, Parent};
use jtac_store::{Roots, Store};

use crate::refuse::Refusal;
use crate::repo::{Head, RepoError};

/// Past this many commits the root records the ends of the range.
pub const COVERS_BOUND: usize = 256;

/// What the root records about its place in history.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Chain {
    /// Oldest first, ending at HEAD.
    pub covers: Covers,
    /// The covered ancestors the walk stopped at, with their roots.
    pub parents: Vec<Parent>,
}

/// Commits with a root in this clone that are proper ancestors of HEAD. A root for HEAD
/// itself, for a descendant, or for a commit this clone lacks would make HEAD uninteresting
/// to rev-list or stop it (`a_root_for_a_descendant_or_a_missing_commit_does_not_block_head`).
pub fn covered(repo: &Path, roots: &Roots, head: &Head) -> Result<BTreeSet<GitOid>, Refusal> {
    let listed: BTreeSet<GitOid> = roots
        .listed
        .iter()
        .map(|l| l.commit)
        .filter(|c| *c != head.commit)
        .collect();
    if listed.is_empty() {
        return Ok(listed);
    }
    let out = git(repo, &["rev-list", &head.commit.hex(), "--"])
        .map_err(|e| Refusal::repository(&RepoError::Git(e)))?;
    let ancestors = String::from_utf8_lossy(&out)
        .lines()
        .map(|hex| oid(head, hex))
        .collect::<Result<BTreeSet<GitOid>, Refusal>>()?;
    Ok(listed.intersection(&ancestors).copied().collect())
}

/// The nearest first-parent ancestor with a root, as hex: the base the rules diff from.
/// Walked as a plain first-parent list, because `--boundary` would also list a merge's
/// second parent, and a covered branch tip there would hide what the first-parent line
/// changed (`the_nearest_covered_ancestor_is_on_the_first_parent_line_or_none_at_all`).
pub fn nearest(
    repo: &Path,
    covered: &BTreeSet<GitOid>,
    head: &Head,
) -> Result<Option<String>, Refusal> {
    if covered.is_empty() {
        return Ok(None);
    }
    let out = git(
        repo,
        &["rev-list", "--first-parent", &head.commit.hex(), "--"],
    )
    .map_err(|e| Refusal::repository(&RepoError::Git(e)))?;
    for hex in String::from_utf8_lossy(&out).lines().skip(1) {
        if covered.contains(&oid(head, hex)?) {
            return Ok(Some(hex.to_owned()));
        }
    }
    Ok(None)
}

/// `covers` and `parents` for a root on `head` whose rules diffed from `base`: every commit
/// reachable from HEAD but from no covered commit and not from `base`, oldest first.
pub fn chain(
    repo: &Path,
    store: &Store,
    covered: &BTreeSet<GitOid>,
    head: &Head,
    base: &str,
) -> Result<Chain, Refusal> {
    let base = (base != head.commit.hex()).then_some(base);
    let lines = walk(repo, &["--topo-order"], head, covered, base)?;
    let mut parents = Vec::new();
    let mut covers = Vec::new();
    for line in &lines {
        match line.strip_prefix('-') {
            Some(hex) => {
                let commit = oid(head, hex)?;
                if covered.contains(&commit) {
                    let stored = store.root(&commit).map_err(Refusal::store)?;
                    let root = stored
                        .ok_or_else(|| Refusal::internal(format!("{commit} lost its root")))?
                        .root
                        .id()
                        .map_err(Refusal::internal)?;
                    parents.push(Parent { commit, root });
                }
            }
            None => covers.push(oid(head, line)?),
        }
    }
    covers.reverse();
    let covers = match covers.first() {
        Some(first) if covers.len() > COVERS_BOUND => Covers::Range {
            base: *first,
            tip: head.commit,
        },
        _ => Covers::List(covers),
    };
    Ok(Chain { covers, parents })
}

/// `covers` and `parents` for a root with `from` (DESIGN.md section 15): the walk stops at
/// `base` alone, so `covers` is every commit reachable from HEAD and not from `base`, the
/// diff the verdict judged, and `parents` is `base`'s root alone, never one of the head's
/// chain, which a root published or withheld later must not change. An empty `parents` is
/// the visible break it always was. Past the bound the range begins at `base` itself.
pub fn from_chain(
    repo: &Path,
    store: &Store,
    head: &Head,
    base: &GitOid,
) -> Result<Chain, Refusal> {
    let lines = walk(
        repo,
        &["--topo-order"],
        head,
        &BTreeSet::new(),
        Some(&base.hex()),
    )?;
    let mut covers: Vec<GitOid> = lines
        .iter()
        .filter(|l| !l.starts_with('-'))
        .map(|l| oid(head, l))
        .collect::<Result<_, _>>()?;
    covers.reverse();
    let covers = match covers.len() > COVERS_BOUND {
        true => Covers::Range {
            base: *base,
            tip: head.commit,
        },
        false => Covers::List(covers),
    };
    let parents = match store.root(base).map_err(Refusal::store)? {
        Some(stored) => vec![Parent {
            commit: *base,
            root: stored.root.id().map_err(Refusal::internal)?,
        }],
        None => Vec::new(),
    };
    Ok(Chain { covers, parents })
}

/// `rev-list --boundary HEAD` with every covered commit and `base` uninteresting, fed on
/// stdin so ten thousand roots are not ten thousand arguments.
fn walk(
    repo: &Path,
    flags: &[&str],
    head: &Head,
    covered: &BTreeSet<GitOid>,
    base: Option<&str>,
) -> Result<Vec<String>, Refusal> {
    let mut args = vec!["rev-list", "--boundary", "--stdin"];
    args.extend(flags);
    let mut stdin = head.commit.hex();
    stdin.push('\n');
    for c in covered
        .iter()
        .map(GitOid::hex)
        .chain(base.map(str::to_owned))
    {
        stdin.push('^');
        stdin.push_str(&c);
        stdin.push('\n');
    }
    let out = git_with(repo, &args, Some(stdin.as_bytes()), &[])
        .map_err(|e| Refusal::repository(&RepoError::Git(e)))?;
    Ok(String::from_utf8_lossy(&out)
        .lines()
        .map(str::to_owned)
        .collect())
}

fn oid(head: &Head, hex: &str) -> Result<GitOid, Refusal> {
    let prefix = match head.commit {
        GitOid::Sha1(_) => "sha1",
        GitOid::Sha256(_) => "sha256",
    };
    format!("{prefix}:{hex}")
        .parse()
        .map_err(|_| Refusal::internal(format!("rev-list printed `{hex}`")))
}
