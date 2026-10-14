//! The ref layer: one parentless commit per root at `refs/jtac/roots/<xx>/<commit>`, its tree
//! `root.json` and `root.sig`. Written with plumbing so no commit hook, template, or editor
//! runs (`update-ref` still fires `reference-transaction`); `--no-gpg-sign` because
//! `commit-tree` honours `commit.gpgsign` and a gpg prompt would hang every root write;
//! `i18n.commitEncoding` pinned because an `encoding` header would change the commit id.

use jtac_checks::git::{GitError, git, git_output, git_with};
use jtac_core::{GitOid, Root, from_canonical};

use crate::roots::StoredRoot;
use crate::{Store, StoreError};

const NAMESPACE: &str = "refs/jtac/roots/";
const AUTHOR: &str = "jtac";

/// The ref that holds the root for `commit`.
pub fn ref_name(commit: &GitOid) -> String {
    let hex = commit.hex();
    format!("{NAMESPACE}{}/{hex}", &hex[..2])
}

/// Write the root commit and create the ref. Returns the root commit's hex. The commit is a
/// function of the root and the signature alone: fixed author, dated the root's `ts`. The
/// ref is created, never replaced: a ref that already holds a root is `Exists`
/// (`a_root_ref_is_created_never_replaced`). Refuses a root that would not read back: one
/// that fails `validate`, names a commit this repository does not have, or is dated before
/// 1970, which git cannot write.
pub(crate) fn write(store: &Store, root: &Root, sig: &[u8]) -> Result<String, StoreError> {
    let repo = store.repo();
    let invalid = |what: String| StoreError::Invalid { what };
    root.validate().map_err(|e| invalid(e.to_string()))?;
    let seconds = root.ts.timestamp().as_second();
    if seconds < 0 {
        return Err(invalid(format!("{} is before 1970", root.ts)));
    }
    let kind = git(repo, &["cat-file", "-t", &root.commit.hex()])
        .ok()
        .map(trimmed)
        .transpose()?;
    if kind.as_deref() != Some("commit") {
        return Err(invalid(format!(
            "{} is not a commit in this repository",
            root.commit
        )));
    }
    let blob = |bytes: &[u8]| -> Result<String, StoreError> {
        let out = git_with(
            repo,
            &["hash-object", "-w", "--no-filters", "--stdin"],
            Some(bytes),
            &[],
        )?;
        trimmed(out)
    };
    let json = blob(&root.canonical_bytes()?)?;
    let sig = blob(sig)?;
    let listing = format!("100644 blob {json}\troot.json\n100644 blob {sig}\troot.sig\n");
    let tree = trimmed(git_with(repo, &["mktree"], Some(listing.as_bytes()), &[])?)?;
    let date = format!("@{seconds} +0000");
    let env = [
        ("GIT_AUTHOR_NAME", AUTHOR),
        ("GIT_AUTHOR_EMAIL", root.signer.principal.as_str()),
        ("GIT_AUTHOR_DATE", date.as_str()),
        ("GIT_COMMITTER_NAME", AUTHOR),
        ("GIT_COMMITTER_EMAIL", root.signer.principal.as_str()),
        ("GIT_COMMITTER_DATE", date.as_str()),
    ];
    let message = root.id()?.to_string();
    let commit = trimmed(git_with(
        repo,
        &[
            "-c",
            "i18n.commitEncoding=UTF-8",
            "commit-tree",
            "--no-gpg-sign",
            "-m",
            &message,
            &tree,
        ],
        None,
        &env,
    )?)?;
    let name = ref_name(&root.commit);
    match git(repo, &["update-ref", &name, &commit, ""]) {
        Ok(_) => Ok(commit),
        Err(GitError::Failed { .. }) if read(store, &root.commit)?.is_some() => {
            Err(StoreError::Exists { name })
        }
        Err(e) => Err(e.into()),
    }
}

/// The root at `commit`'s ref, or `None` when there is no such ref. A ref that exists but
/// does not hold a root for `commit` is refused, never a miss.
pub(crate) fn read(store: &Store, commit: &GitOid) -> Result<Option<StoredRoot>, StoreError> {
    let repo = store.repo();
    let name = ref_name(commit);
    let oid = match git(
        repo,
        &[
            "rev-parse",
            "--verify",
            "--quiet",
            "--end-of-options",
            &name,
        ],
    ) {
        Ok(out) => trimmed(out)?,
        Err(GitError::Failed {
            status: Some(1), ..
        }) => return Ok(None),
        Err(e) => return Err(e.into()),
    };
    let refuse = |what: String| StoreError::Ref {
        name: name.clone(),
        what,
    };
    let kind = trimmed(git(repo, &["cat-file", "-t", &oid])?)?;
    if kind != "commit" {
        return Err(refuse(format!(
            "the ref points at a {kind}, not a root commit"
        )));
    }
    let blob = |file: &str| git(repo, &["cat-file", "blob", &format!("{oid}:{file}")]);
    let bytes = blob("root.json")?;
    let root: Root = from_canonical(&bytes).map_err(|e| refuse(e.to_string()))?;
    root.validate().map_err(|e| refuse(e.to_string()))?;
    if root.commit != *commit {
        return Err(refuse(format!("the root is for {}", root.commit)));
    }
    let sig = blob("root.sig")?;
    Ok(Some(StoredRoot { root, sig }))
}

/// One ref under the namespace.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Listed {
    /// The commit the root is for, from the ref name.
    pub commit: GitOid,
    /// The root commit the ref points at.
    pub root_commit: GitOid,
}

impl Store {
    /// Every root ref in this clone, in ref order.
    pub fn roots(&self) -> Result<Vec<Listed>, StoreError> {
        let repo = self.repo();
        let format = trimmed(git(repo, &["rev-parse", "--show-object-format"])?)?;
        let out = trimmed(git(
            repo,
            &[
                "for-each-ref",
                "--format=%(refname) %(objectname)",
                NAMESPACE,
            ],
        )?)?;
        let oid = |hex: &str| -> Result<GitOid, StoreError> {
            format!("{format}:{hex}")
                .parse()
                .map_err(|_| StoreError::Output {
                    what: format!("{hex} is not a {format} object id"),
                })
        };
        out.lines()
            .map(|line| {
                let (name, object) = line.split_once(' ').ok_or_else(|| StoreError::Output {
                    what: format!("for-each-ref line {line:?}"),
                })?;
                let commit = name
                    .strip_prefix(NAMESPACE)
                    .and_then(|tail| tail.split_once('/'))
                    .filter(|(shard, hex)| {
                        shard.len() == 2 && hex.starts_with(shard) && !hex.contains('/')
                    })
                    .and_then(|(_, hex)| oid(hex).ok())
                    .ok_or_else(|| StoreError::Ref {
                        name: name.to_owned(),
                        what: "the ref is not <xx>/<commit> for this repository's format".into(),
                    })?;
                Ok(Listed {
                    commit,
                    root_commit: oid(object)?,
                })
            })
            .collect()
    }

    /// Push the root refs for `commits` to `remote`, never forced. Each ref's outcome is
    /// reported; a rejection means the remote already holds a root for that commit. An empty
    /// list pushes nothing: a bare `git push` would send the current branch.
    pub fn push(&self, remote: &str, commits: &[GitOid]) -> Result<Vec<Pushed>, StoreError> {
        let mut distinct: Vec<&GitOid> = Vec::new();
        for c in commits {
            if !distinct.contains(&c) {
                distinct.push(c);
            }
        }
        if distinct.is_empty() {
            return Ok(Vec::new());
        }
        let specs: Vec<String> = distinct
            .iter()
            .map(|c| {
                let name = ref_name(c);
                format!("{name}:{name}")
            })
            .collect();
        let mut args = vec!["push", "--porcelain", "--", remote];
        args.extend(specs.iter().map(String::as_str));
        let out = git_output(self.repo(), &args, None, &[])?;
        let text = String::from_utf8_lossy(&out.stdout);
        let pushed: Vec<Pushed> = text.lines().filter_map(porcelain).collect();
        if pushed.len() != distinct.len() {
            let stderr = String::from_utf8_lossy(&out.stderr);
            return Err(GitError::Failed {
                argv: std::iter::once("git")
                    .chain(args.iter().copied())
                    .map(str::to_owned)
                    .collect(),
                status: out.status.code(),
                stderr: stderr.lines().next().unwrap_or_default().to_owned(),
            }
            .into());
        }
        Ok(pushed)
    }
}

/// What `push --porcelain` said about one ref.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pushed {
    /// The ref.
    pub name: String,
    /// What happened to it.
    pub outcome: PushOutcome,
}

/// One ref's push result.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PushOutcome {
    /// The remote now has it.
    Sent,
    /// The remote already had this exact root.
    UpToDate,
    /// The remote refused it, most often because it holds a different root for the commit.
    Rejected {
        /// Git's summary, for example `[rejected] (fetch first)`.
        summary: String,
    },
}

// `<flag>\t<from>:<to>\t<summary>`; a line without two tabs is the `To <url>` or `Done` line.
fn porcelain(line: &str) -> Option<Pushed> {
    let mut parts = line.splitn(3, '\t');
    let flag = parts.next()?;
    let refs = parts.next()?;
    let summary = parts.next()?;
    let name = refs.split_once(':').map_or(refs, |(_, to)| to).to_owned();
    let outcome = match flag {
        "=" => PushOutcome::UpToDate,
        "!" => PushOutcome::Rejected {
            summary: summary.to_owned(),
        },
        _ => PushOutcome::Sent,
    };
    Some(Pushed { name, outcome })
}

fn trimmed(out: Vec<u8>) -> Result<String, StoreError> {
    String::from_utf8(out)
        .map(|s| s.trim().to_owned())
        .map_err(|_| StoreError::Output {
            what: "git printed bytes that are not UTF-8".into(),
        })
}
