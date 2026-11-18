//! Facts about the repository under test, read through git and decided by no one here
//! (DESIGN.md section 11.1 preflight). HEAD is resolved once and every later call names its
//! hex, so a ref moving under the run cannot swap the commit the rules read. The base rule is
//! section 10.3's: full ref names only, so a tag spelled `main` never shadows the branch
//! (`a_tag_named_main_does_not_shadow_the_branch`).

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use jtac_checks::diff::git::{GitError, git, merge_base};
use jtac_core::GitOid;
use jtac_store::{StoreError, git_objects};
use serde::{Deserialize, Serialize};

/// HEAD as the verdict names it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Head {
    /// The commit under test.
    pub commit: GitOid,
    /// Its tree.
    pub tree: GitOid,
}

/// The tree-ish the diff rules read from, and how it was chosen.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Base {
    /// Hex, as git prints it.
    pub commit: String,
    /// `merge base with origin/main`, `first parent of HEAD (...)`, `the empty tree (...)`.
    pub from: String,
}

/// What stopped a fact from being read.
#[derive(Debug, thiserror::Error)]
pub enum RepoError {
    /// git did not start or exited non-zero.
    #[error(transparent)]
    Git(#[from] GitError),
    /// git printed an object id in a format this build does not read.
    #[error("git printed an object id this build cannot read: {text}")]
    Oid {
        /// The text.
        text: String,
    },
    /// The path at HEAD is a symlink or a directory.
    #[error("`{path}` at HEAD is not a regular file")]
    NotAFile {
        /// Repository-relative path.
        path: String,
    },
    /// The clone lacks history the base rule needs.
    #[error("the history is shallow: {what}")]
    Shallow {
        /// What could not be read.
        what: String,
    },
    /// The path names a file this clone does not hold.
    #[error("`{path}` at HEAD is object {id}, which this clone does not hold")]
    Missing {
        /// Repository-relative path.
        path: String,
        /// The object, as hex.
        id: String,
    },
    /// git answered in a shape this build does not read.
    #[error("git's answer does not read: {what}")]
    Answer {
        /// What was wrong with it.
        what: String,
    },
}

fn text(out: &[u8]) -> String {
    String::from_utf8_lossy(out).trim().to_owned()
}

/// `None` only on the exit git documents for "no such thing" (`absent`): 1 for the `--quiet`
/// forms and `merge-base` with no common ancestor, 128 for `@{upstream}` on a branch without
/// one. Any other failure is fatal and propagates, so a missing object never reads as absence.
pub(crate) fn probe(repo: &Path, args: &[&str], absent: i32) -> Result<Option<String>, RepoError> {
    match git(repo, args) {
        Ok(out) => Ok(Some(text(&out))),
        Err(GitError::Failed {
            status: Some(status),
            ..
        }) if status == absent => Ok(None),
        Err(e) => Err(e.into()),
    }
}

/// Whether `info/grafts` exists: a graft rewrites the parents git shows, so `check`, the
/// hook, and `status` refuse rather than read a history the remote will not see
/// (`a_replace_graft_does_not_hide_the_change`, `a_grafts_file_refuses_the_hook_and_status`).
pub fn grafted(repo: &Path) -> Result<Option<PathBuf>, RepoError> {
    let out = git(repo, &["rev-parse", "--git-path", "info/grafts"])?;
    let path = repo.join(text(&out));
    Ok(path.exists().then_some(path))
}

/// One git config value, `None` when unset.
pub fn config(repo: &Path, key: &str) -> Result<Option<String>, RepoError> {
    probe(repo, &["config", "--get", key], 1)
}

/// The full ref name of the current branch's upstream, when it has one.
/// The remote a bare `git push` of `short`'s branch goes to, as git picks it: the branch's
/// push remote, the push default, its upstream remote, else `origin`.
pub fn push_remote(repo: &Path, short: Option<&str>) -> Result<String, RepoError> {
    for key in [
        short.map(|b| format!("branch.{b}.pushRemote")),
        Some("remote.pushDefault".to_owned()),
        short.map(|b| format!("branch.{b}.remote")),
    ]
    .into_iter()
    .flatten()
    {
        if let Some(name) = config(repo, &key)? {
            return Ok(name);
        }
    }
    Ok("origin".to_owned())
}

pub fn upstream(repo: &Path) -> Result<Option<String>, RepoError> {
    probe(
        repo,
        &["rev-parse", "--symbolic-full-name", "@{upstream}"],
        128,
    )
}

/// Commits reachable from HEAD and not from `upstream`, oldest first.
pub fn ahead(repo: &Path, head: &Head, upstream: &str) -> Result<Vec<GitOid>, RepoError> {
    let not = format!("^{upstream}");
    let commit = head.commit.hex();
    let out = git(
        repo,
        &["rev-list", "--reverse", "--topo-order", &commit, &not, "--"],
    )?;
    let prefix = match head.commit {
        GitOid::Sha1(_) => "sha1",
        GitOid::Sha256(_) => "sha256",
    };
    text(&out)
        .lines()
        .map(|hex| {
            format!("{prefix}:{hex}")
                .parse()
                .map_err(|_| RepoError::Oid {
                    text: hex.to_owned(),
                })
        })
        .collect()
}

/// The work tree root above `cwd`, bytes kept as git printed them minus the newline.
pub fn root(cwd: &Path) -> Result<PathBuf, RepoError> {
    let mut out = git(cwd, &["rev-parse", "--show-toplevel"])?;
    if out.last() == Some(&b'\n') {
        out.pop();
    }
    #[cfg(unix)]
    let path = <OsString as std::os::unix::ffi::OsStringExt>::from_vec(out);
    #[cfg(not(unix))]
    let path = OsString::from(String::from_utf8_lossy(&out).into_owned());
    Ok(PathBuf::from(path))
}

/// HEAD's commit and tree in the repository's object format.
pub fn head(repo: &Path) -> Result<Head, RepoError> {
    peel(repo, &format(repo)?, "HEAD")
}

/// `rev` peeled to a commit, and that commit's tree.
pub fn peel(repo: &Path, format: &str, rev: &str) -> Result<Head, RepoError> {
    let commit = resolve(repo, format, &format!("{rev}^{{commit}}"))?;
    let tree = resolve(repo, format, &format!("{}^{{tree}}", commit.hex()))?;
    Ok(Head { commit, tree })
}

/// The object format as git names it: `sha1` or `sha256`, the prefix a [`GitOid`] takes.
pub fn format(repo: &Path) -> Result<String, RepoError> {
    Ok(text(&git(repo, &["rev-parse", "--show-object-format"])?))
}

/// `hex` as an object id in `format`.
pub fn oid(format: &str, hex: &str) -> Result<GitOid, RepoError> {
    let full = format!("{format}:{hex}");
    full.parse().map_err(|_| RepoError::Oid { text: full })
}

fn resolve(repo: &Path, format: &str, spec: &str) -> Result<GitOid, RepoError> {
    let hex = text(&git(
        repo,
        &["rev-parse", "--verify", "--end-of-options", spec],
    )?);
    oid(format, &hex)
}

/// Why the work tree is not HEAD's tree, or `None`. `git status` alone is not enough: an
/// index entry marked skip-worktree or assume-unchanged hides a modified file from it, and
/// `info/exclude` or `core.excludesFile` hides an untracked one, neither of which the tree
/// under test carries (`a_tree_that_status_calls_clean_can_still_differ_from_head`).
pub fn dirty(repo: &Path) -> Result<Option<String>, RepoError> {
    let out = git(
        repo,
        &[
            "--no-optional-locks",
            "status",
            "--porcelain",
            "-z",
            "--untracked-files=all",
        ],
    )?;
    if !out.is_empty() {
        return Ok(Some("`git status` shows uncommitted changes".to_owned()));
    }
    let entries = git(repo, &["ls-files", "-v", "-z"])?;
    let hidden = entries
        .split(|b| *b == 0)
        .find(|e| matches!(e.first(), Some(b'S' | b'a'..=b'z')));
    if let Some(entry) = hidden {
        let path = String::from_utf8_lossy(entry.get(2..).unwrap_or_default());
        return Ok(Some(format!(
            "`{path}` is marked skip-worktree or assume-unchanged, so `git status` does not \
             show its changes"
        )));
    }
    let others = git(
        repo,
        &["ls-files", "-o", "-z", "--exclude-per-directory=.gitignore"],
    )?;
    Ok(others.split(|b| *b == 0).find(|e| !e.is_empty()).map(|e| {
        format!(
            "`{}` is untracked and hidden by a local exclude, not by `.gitignore`",
            String::from_utf8_lossy(e)
        )
    }))
}

/// What a tree holds at a path, as git resolves it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Entry {
    /// No entry.
    None,
    /// One regular file: its blob id.
    File(String),
    /// Anything else: a link, a gitlink, a directory, more than one entry.
    Other,
}

/// The entry at `path` in `treeish`. The one reading of "a regular file is here", for the
/// gate that loads a policy and for every reader that asks what a tree's policy is, so the
/// two cannot disagree on a tree (`the_policy_of_a_tree_is_what_git_resolves_at_the_path`).
/// git resolves the path; nothing here parses a tree.
pub fn entry_at(repo: &Path, treeish: &str, path: &str) -> Result<Entry, RepoError> {
    let out = git(repo, &["ls-tree", "-z", treeish, "--", path])?;
    let listed = String::from_utf8_lossy(&out);
    let entries: Vec<&str> = listed.split('\0').filter(|e| !e.is_empty()).collect();
    Ok(match entries[..] {
        [] => Entry::None,
        [one] => match one.split(['\t', ' ']).collect::<Vec<_>>()[..] {
            ["100644" | "100755", "blob", sha, ..] => Entry::File(sha.to_owned()),
            _ => Entry::Other,
        },
        _ => Entry::Other,
    })
}

/// What a file entry's id names in this clone.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Held {
    /// A blob: its bytes.
    Blob(Vec<u8>),
    /// Another kind of object, whatever the entry's mode says.
    NotBlob,
    /// Nothing this clone holds.
    Missing,
}

/// What each id names. The one read of a file's bytes, for the gate that loads a policy and
/// every reader that asks what a tree's is: git names the kind and peels nothing, so a tag
/// of a blob is a file to neither
/// (`a_file_entry_that_names_no_blob_is_a_file_to_neither_the_gate_nor_the_reader`).
pub fn held(repo: &Path, ids: &[String]) -> Result<Vec<Held>, RepoError> {
    let read = git_objects(repo, ids).map_err(|e| match e {
        StoreError::Git(e) => RepoError::Git(e),
        other => RepoError::Answer {
            what: other.to_string(),
        },
    })?;
    Ok(read
        .into_iter()
        .map(|object| match object {
            Some(o) if o.kind == "blob" => Held::Blob(o.bytes),
            Some(_) => Held::NotBlob,
            None => Held::Missing,
        })
        .collect())
}

/// The blob at `path` in `head`'s tree as `(sha, bytes)`; `None` when there is no entry.
pub fn blob_at(
    repo: &Path,
    head: &Head,
    path: &str,
) -> Result<Option<(String, Vec<u8>)>, RepoError> {
    let path = || path.to_owned();
    match entry_at(repo, &head.commit.hex(), &path())? {
        Entry::None => Ok(None),
        Entry::File(sha) => match held(repo, std::slice::from_ref(&sha))?.pop() {
            Some(Held::Blob(bytes)) => Ok(Some((sha, bytes))),
            Some(Held::NotBlob) => Err(RepoError::NotAFile { path: path() }),
            Some(Held::Missing) | None => Err(RepoError::Missing {
                path: path(),
                id: sha,
            }),
        },
        Entry::Other => Err(RepoError::NotAFile { path: path() }),
    }
}

/// Whether the file at `path` in the work tree would hash to `sha` after git's clean filters,
/// so `core.autocrlf` alone never reads as a difference. Anything but a regular file there
/// (missing, a directory, a dangling link) is a difference; a filter that fails is git's
/// error and propagates.
pub fn on_disk_matches(repo: &Path, path: &str, sha: &str) -> Result<bool, RepoError> {
    if !std::fs::metadata(repo.join(path)).is_ok_and(|m| m.is_file()) {
        return Ok(false);
    }
    Ok(text(&git(repo, &["hash-object", "--", path])?) == sha)
}

/// What the remote's default branch holds of HEAD's history (DESIGN.md section 10.3,
/// decision 69): the merge base of HEAD with `origin_default`, and that ref's short name.
/// Never the upstream, which for a pushed branch is the branch itself, nor a local branch,
/// whose unpushed commits no gate has seen. `None` without the ref or a common ancestor.
/// Nothing is fetched: the ref is the clone's, as stale or as moved as the clone left it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Landed {
    /// The merge base, hex.
    pub commit: String,
    /// `origin/main`.
    pub shown: String,
}

/// `Landed` for `head`, read from this clone alone.
pub fn landed(repo: &Path, head: &Head) -> Result<Option<Landed>, RepoError> {
    let Some((full, shown)) = origin_default(repo)? else {
        return Ok(None);
    };
    let found = probe(repo, &["merge-base", &full, &head.commit.hex()], 1)?;
    Ok(found.map(|commit| Landed { commit, shown }))
}

/// The branch `refs/remotes/origin/HEAD` names, full and short, when that exact ref is in
/// this clone: the one read of origin's default branch. A symref left pointing at a branch
/// the remote renamed is no name, never a git failure; `show-ref --verify` matches the ref
/// alone, where `rev-parse` would find a tag of that name; a ref whose object is missing
/// refuses (`a_dangling_origin_head_reads_as_no_default_branch` holds all three).
fn origin_default(repo: &Path) -> Result<Option<(String, String)>, RepoError> {
    let name = ["symbolic-ref", "--quiet", "refs/remotes/origin/HEAD"];
    let Some(full) = probe(repo, &name, 1)? else {
        return Ok(None);
    };
    if probe(repo, &["show-ref", "--verify", "--quiet", &full], 1)?.is_none() {
        return Ok(None);
    }
    let shown = full
        .strip_prefix("refs/remotes/")
        .unwrap_or(&full)
        .to_owned();
    Ok(Some((full, shown)))
}

/// Why `landed` has nothing to read, where a note can say what to run. Nothing is said of a
/// clone with no remote, or of an `origin` that holds no branch yet.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Unnamed {
    /// `origin` holds branches and `origin/HEAD` names none of them.
    Head,
    /// The same in a clone whose fetch lines name single branches (`clone --single-branch`),
    /// where `set-head --auto` fails until the default branch is fetched.
    Narrow,
    /// No remote is named `origin`; the only remote, when there is one.
    Remote(Option<String>),
}

/// `Unnamed` for this clone, `None` where the default branch is named. Names alone are
/// listed: the default `for-each-ref` format reads each object.
pub fn unnamed(repo: &Path) -> Result<Option<Unnamed>, RepoError> {
    if origin_default(repo)?.is_some() {
        return Ok(None);
    }
    let listed = text(&git(repo, &["remote"])?);
    let remotes: Vec<&str> = listed.lines().collect();
    match remotes[..] {
        [] => return Ok(None),
        _ if remotes.contains(&"origin") => {}
        [only] => return Ok(Some(Unnamed::Remote(Some(only.to_owned())))),
        _ => return Ok(Some(Unnamed::Remote(None))),
    }
    let names = ["for-each-ref", "--count=1", "--format=%(refname)"];
    let held = git(repo, &[&names[..], &["refs/remotes/origin/"]].concat())?;
    if text(&held).is_empty() {
        return Ok(None);
    }
    let fetch = ["config", "--get-all", "remote.origin.fetch"];
    let wide = probe(repo, &fetch, 1)?.is_none_or(|lines| lines.contains("refs/heads/*"));
    Ok(Some(if wide { Unnamed::Head } else { Unnamed::Narrow }))
}

/// The base the rules diff from (DESIGN.md section 10.3): the merge base of HEAD with
/// `explicit`; else the `nearest` covered ancestor when it descends from what `landed`
/// names, or nothing is named, and the `landed` merge base when it does not
/// (`a_branch_cut_from_a_merge_with_no_root_is_judged_on_its_own_commits`); else the merge
/// base with the upstream, else with the default branch. When that is HEAD itself, or
/// nothing names a candidate, the rules judge the last commit: the first parent, or the
/// empty tree for a parentless HEAD. An explicit rev that does not resolve or shares no
/// history is the caller's refusal.
pub fn base(
    repo: &Path,
    head: &Head,
    explicit: Option<&str>,
    nearest: Option<&str>,
    landed: Option<&Landed>,
) -> Result<Base, RepoError> {
    let commit = head.commit.hex();
    let (candidate, reason) = match (explicit, nearest) {
        (Some(rev), _) => {
            let spec = format!("{rev}^{{commit}}");
            git(repo, &["rev-parse", "--verify", "--end-of-options", &spec])?;
            let base = merge_base(repo, rev, &commit)?;
            (Some(base), format!("--base {rev}"))
        }
        (None, Some(covered)) => {
            let own = |l: &Landed| {
                probe(
                    repo,
                    &["merge-base", "--is-ancestor", &l.commit, covered],
                    1,
                )
            };
            match landed {
                Some(l) if own(l)?.is_none() => (
                    Some(l.commit.clone()),
                    format!("merge base with {}", l.shown),
                ),
                _ => (
                    Some(covered.to_owned()),
                    "nearest covered ancestor".to_owned(),
                ),
            }
        }
        (None, None) => match candidate(repo)? {
            Some((full, shown)) => match probe(repo, &["merge-base", &full, &commit], 1)? {
                Some(base) => (Some(base), format!("merge base with {shown}")),
                None if shallow(repo)? => {
                    return Err(RepoError::Shallow {
                        what: format!("the merge base with {shown} is not in this clone"),
                    });
                }
                None => (None, format!("no common ancestor with {shown}")),
            },
            None => (None, "no upstream, no main or master branch".to_owned()),
        },
    };
    match candidate {
        Some(base) if base != commit => Ok(Base {
            commit: base,
            from: reason,
        }),
        Some(_) => last_commit(repo, &commit, &format!("{reason} is at HEAD")),
        None => last_commit(repo, &commit, &reason),
    }
}

/// The first parent, or HEAD itself when it has none: a root commit changes nothing, and
/// judging it against the empty tree would block on its own policy file. The parent is read
/// from the commit object itself, because `rev-list` hides it at a shallow boundary, and a
/// parent the clone lacks is that boundary, never a root
/// (`a_shallow_clone_refuses_instead_of_judging_nothing`).
fn last_commit(repo: &Path, commit: &str, why: &str) -> Result<Base, RepoError> {
    let object = git(repo, &["cat-file", "commit", commit])?;
    let parent = String::from_utf8_lossy(&object)
        .lines()
        .take_while(|l| !l.is_empty())
        .find_map(|l| l.strip_prefix("parent ").map(str::to_owned));
    let Some(parent) = parent else {
        return Ok(Base {
            commit: commit.to_owned(),
            from: "HEAD itself (HEAD has no parent, so there is no change to judge)".to_owned(),
        });
    };
    let spec = format!("{parent}^{{commit}}");
    match git(repo, &["rev-parse", "--verify", "--quiet", &spec]) {
        Ok(_) => Ok(Base {
            commit: parent,
            from: format!("first parent of HEAD ({why})"),
        }),
        Err(GitError::Failed { .. }) => Err(RepoError::Shallow {
            what: format!("HEAD's parent {parent} is not in this clone"),
        }),
        Err(e) => Err(e.into()),
    }
}

/// Whether `.git/shallow` marks commits, so missing history is a refusal, never absence
/// (`a_shallow_clone_refuses_instead_of_judging_nothing`); a writing `check` refuses here
/// outright, since a mark written by hand reads the same as a depth-limited clone
/// (`a_shallow_mark_real_or_forged_gets_no_root`).
pub fn shallow(repo: &Path) -> Result<bool, RepoError> {
    Ok(text(&git(repo, &["rev-parse", "--is-shallow-repository"])?) == "true")
}

/// The ref to take the merge base with, full name for git and short name for people.
fn candidate(repo: &Path) -> Result<Option<(String, String)>, RepoError> {
    let remote = |full: String| {
        let shown = full
            .strip_prefix("refs/remotes/")
            .unwrap_or(&full)
            .to_owned();
        (full, shown)
    };
    if let Some(full) = probe(
        repo,
        &["rev-parse", "--symbolic-full-name", "@{upstream}"],
        128,
    )? {
        return Ok(Some(remote(full)));
    }
    if let Some(named) = origin_default(repo)? {
        return Ok(Some(named));
    }
    for name in ["main", "master"] {
        let full = format!("refs/heads/{name}");
        if probe(repo, &["rev-parse", "--verify", "--quiet", &full], 1)?.is_some() {
            return Ok(Some((full, name.to_owned())));
        }
    }
    Ok(None)
}
