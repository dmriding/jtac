//! The ref layer: one parentless commit per root at `refs/jtac/roots/<xx>/<commit>`, its tree
//! `root.json` and `root.sig`. Written with plumbing so no commit hook, template, or editor
//! runs (`update-ref` still fires `reference-transaction`); `--no-gpg-sign` because
//! `commit-tree` honours `commit.gpgsign` and a gpg prompt would hang every root write;
//! `i18n.commitEncoding` pinned because an `encoding` header would change the commit id.

use jtac_checks::git::{GitError, git, git_output, git_with, quoted};
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
    let invalid = |what: String| StoreError::Invalid { what };
    root.validate().map_err(|e| invalid(e.to_string()))?;
    if kind_of(store, &root.commit.hex())?.as_deref() != Some("commit") {
        return Err(invalid(format!(
            "{} is not a commit in this repository",
            root.commit
        )));
    }
    let files = [
        ("root.json", root.canonical_bytes()?),
        ("root.sig", sig.to_vec()),
    ];
    let commit = commit_files(
        store,
        &files,
        &root.signer.principal,
        root.ts,
        &root.id()?.to_string(),
    )?;
    let name = ref_name(&root.commit);
    match git(store.repo(), &["update-ref", &name, &commit, ""]) {
        Ok(_) => Ok(commit),
        Err(GitError::Failed { .. }) if read(store, &root.commit)?.is_some() => {
            Err(StoreError::Exists { name })
        }
        Err(e) => Err(e.into()),
    }
}

/// `cat-file -t` of `hex`, `None` when the repository has no such object.
pub(crate) fn kind_of(store: &Store, hex: &str) -> Result<Option<String>, StoreError> {
    git(store.repo(), &["cat-file", "-t", hex])
        .ok()
        .map(trimmed)
        .transpose()
}

/// A parentless commit over `files`, a function of their bytes alone: fixed author, dated
/// `ts`, `message` the document's id. Refuses a date before 1970, which git cannot write.
pub(crate) fn commit_files(
    store: &Store,
    files: &[(&str, Vec<u8>)],
    principal: &str,
    ts: jtac_core::Ts,
    message: &str,
) -> Result<String, StoreError> {
    let repo = store.repo();
    let seconds = ts.timestamp().as_second();
    if seconds < 0 {
        return Err(StoreError::Invalid {
            what: format!("{ts} is before 1970"),
        });
    }
    let mut listing = String::new();
    for (name, bytes) in files {
        let out = git_with(
            repo,
            &["hash-object", "-w", "--no-filters", "--stdin"],
            Some(bytes),
            &[],
        )?;
        listing.push_str(&format!("100644 blob {}\t{name}\n", trimmed(out)?));
    }
    let tree = trimmed(git_with(repo, &["mktree"], Some(listing.as_bytes()), &[])?)?;
    let date = format!("@{seconds} +0000");
    let env = [
        ("GIT_AUTHOR_NAME", AUTHOR),
        ("GIT_AUTHOR_EMAIL", principal),
        ("GIT_AUTHOR_DATE", date.as_str()),
        ("GIT_COMMITTER_NAME", AUTHOR),
        ("GIT_COMMITTER_EMAIL", principal),
        ("GIT_COMMITTER_DATE", date.as_str()),
    ];
    trimmed(git_with(
        repo,
        &[
            "-c",
            "i18n.commitEncoding=UTF-8",
            "commit-tree",
            "--no-gpg-sign",
            "-m",
            message,
            &tree,
        ],
        None,
        &env,
    )?)
}

/// The root at `commit`'s ref, or `None` when there is no such ref. A ref that exists but
/// does not hold a root for `commit` is refused, never a miss.
pub(crate) fn read(store: &Store, commit: &GitOid) -> Result<Option<StoredRoot>, StoreError> {
    read_at(store, commit, &ref_name(commit))
}

/// `commit`'s root as the ref `name` holds it: its own ref, or `FETCH_HEAD` for a remote's.
fn read_at(store: &Store, commit: &GitOid, name: &str) -> Result<Option<StoredRoot>, StoreError> {
    let Some(doc) = document(store, name, "root")? else {
        return Ok(None);
    };
    let refuse = |what: String| StoreError::Ref {
        name: name.to_owned(),
        what,
    };
    let root: Root = from_canonical(&doc.blob("root.json")?).map_err(|e| refuse(e.to_string()))?;
    root.validate().map_err(|e| refuse(e.to_string()))?;
    if root.commit != *commit {
        return Err(refuse(format!("the root is for {}", root.commit)));
    }
    let sig = doc.blob("root.sig")?;
    Ok(Some(StoredRoot { root, sig }))
}

/// The commit a document ref points at, or `None` when there is no such ref. A ref that
/// points at anything but a commit is refused, never a miss.
pub(crate) fn document<'a>(
    store: &'a Store,
    name: &str,
    what: &str,
) -> Result<Option<Document<'a>>, StoreError> {
    let repo = store.repo();
    let oid = match git(
        repo,
        &["rev-parse", "--verify", "--quiet", "--end-of-options", name],
    ) {
        Ok(out) => trimmed(out)?,
        Err(GitError::Failed {
            status: Some(1), ..
        }) => return Ok(None),
        Err(e) => return Err(e.into()),
    };
    let kind = trimmed(git(repo, &["cat-file", "-t", &oid])?)?;
    if kind != "commit" {
        return Err(StoreError::Ref {
            name: name.to_owned(),
            what: format!("the ref points at a {kind}, not a {what} commit"),
        });
    }
    Ok(Some(Document {
        store,
        name: name.to_owned(),
        oid,
    }))
}

/// A document commit's files.
pub(crate) struct Document<'a> {
    store: &'a Store,
    name: String,
    oid: String,
}

impl Document<'_> {
    /// `file` in the commit's tree; a commit without it is a refused ref, never a miss.
    pub(crate) fn blob(&self, file: &str) -> Result<Vec<u8>, StoreError> {
        match git(
            self.store.repo(),
            &["cat-file", "blob", &format!("{}:{file}", self.oid)],
        ) {
            Ok(bytes) => Ok(bytes),
            Err(GitError::Failed { .. }) => Err(StoreError::Ref {
                name: self.name.clone(),
                what: format!("the commit holds no {file}"),
            }),
            Err(e) => Err(e.into()),
        }
    }
}

/// One ref under the namespace.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Listed {
    /// The commit the root is for, from the ref name.
    pub commit: GitOid,
    /// The root commit the ref points at.
    pub root_commit: GitOid,
}

/// The namespace as read: the root refs, and the names under it that are not
/// `<xx>/<commit>`, a name that is not UTF-8 included. Nobody writes those through jtac, so
/// they are a note, never a reason to stop judging the tip; anyone with push access chooses
/// them, so a note shows a name only when every character is plain, and names no command
/// with a lossy name, which would delete another ref
/// (`roots_skips_a_ref_the_writer_would_not_write_with_a_note`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Roots {
    /// In ref order.
    pub listed: Vec<Listed>,
    /// Full ref names as git printed them, in ref order.
    pub skipped: Vec<Vec<u8>>,
}

impl Roots {
    /// One note per skipped ref, for whichever command prints notes.
    pub fn notes(&self) -> impl Iterator<Item = String> + '_ {
        self.skipped
            .iter()
            .map(|raw| match std::str::from_utf8(raw) {
                Ok(name) if quoted(name, "") == name => format!(
                    "skipped {name}: not <xx>/<commit> under refs/jtac/roots/; `git update-ref -d \
                 {name}` removes it here and `git push <remote> :{name}` on the remote"
                ),
                _ => "skipped a ref under refs/jtac/roots/ whose name this note cannot show; `git \
                  for-each-ref refs/jtac/roots/` prints it, `git update-ref -d <ref>` removes \
                  it here and `git push <remote> :<ref>` on the remote"
                    .to_owned(),
            })
    }
}

impl Store {
    /// Every root ref in this clone, in ref order, and every name skipped.
    pub fn roots(&self) -> Result<Roots, StoreError> {
        let repo = self.repo();
        let format = trimmed(git(repo, &["rev-parse", "--show-object-format"])?)?;
        let out = git(
            repo,
            &[
                "for-each-ref",
                "--format=%(refname) %(objectname)",
                NAMESPACE,
            ],
        )?;
        let oid = |hex: &str| -> Result<GitOid, StoreError> {
            format!("{format}:{hex}")
                .parse()
                .map_err(|_| StoreError::Output {
                    what: format!("{hex} is not a {format} object id"),
                })
        };
        let mut roots = Roots::default();
        for raw in out.split(|b| *b == b'\n').filter(|l| !l.is_empty()) {
            let Ok(line) = std::str::from_utf8(raw) else {
                let name = raw.split(|b| *b == b' ').next().unwrap_or_default();
                roots.skipped.push(name.to_vec());
                continue;
            };
            let (name, object) = line.split_once(' ').ok_or_else(|| StoreError::Output {
                what: format!("for-each-ref line {line:?}"),
            })?;
            let commit = name
                .strip_prefix(NAMESPACE)
                .and_then(|tail| tail.split_once('/'))
                .filter(|(shard, hex)| {
                    shard.len() == 2 && hex.starts_with(shard) && !hex.contains('/')
                })
                .and_then(|(_, hex)| oid(hex).ok());
            match commit {
                Some(commit) => roots.listed.push(Listed {
                    commit,
                    root_commit: oid(object)?,
                }),
                None => roots.skipped.push(name.as_bytes().to_vec()),
            }
        }
        Ok(roots)
    }

    /// The root `remote` holds for `commit`, or `None` when it holds no ref for it: fetched
    /// into `FETCH_HEAD` alone, `--refmap=` keeping a configured `refs/jtac/*` refspec from
    /// replacing the local ref on the way, and read as the local ref is.
    pub fn remote_root(
        &self,
        remote: &str,
        commit: &GitOid,
    ) -> Result<Option<StoredRoot>, StoreError> {
        let name = ref_name(commit);
        match git(
            self.repo(),
            &["ls-remote", "--exit-code", "--", remote, &name],
        ) {
            Ok(_) => {}
            Err(GitError::Failed {
                status: Some(2), ..
            }) => return Ok(None),
            Err(e) => return Err(e.into()),
        }
        git(
            self.repo(),
            &[
                "fetch",
                "--quiet",
                "--no-tags",
                "--refmap=",
                "--",
                remote,
                &name,
            ],
        )?;
        read_at(self, commit, "FETCH_HEAD")
    }

    /// Push the root refs for `commits` to `remote`, never forced, with `env` on the child git
    /// (the `pre-push` hook's `JTAC_HOOK=1`). Each ref's outcome is reported; a rejection means
    /// the remote already holds a root for that commit. An empty list pushes nothing: a bare
    /// `git push` would send the current branch.
    pub fn push(
        &self,
        remote: &str,
        commits: &[GitOid],
        env: &[(&str, &str)],
    ) -> Result<Vec<Pushed>, StoreError> {
        let mut distinct: Vec<&GitOid> = Vec::new();
        for c in commits {
            if !distinct.contains(&c) {
                distinct.push(c);
            }
        }
        if distinct.is_empty() {
            return Ok(Vec::new());
        }
        let names: Vec<String> = distinct.iter().map(|c| ref_name(c)).collect();
        self.push_refs(remote, &names, env)
    }

    /// Push `names` to `remote` as themselves, never forced.
    pub(crate) fn push_refs(
        &self,
        remote: &str,
        names: &[String],
        env: &[(&str, &str)],
    ) -> Result<Vec<Pushed>, StoreError> {
        let specs: Vec<String> = names.iter().map(|name| format!("{name}:{name}")).collect();
        let mut args = vec!["push", "--porcelain", "--", remote];
        args.extend(specs.iter().map(String::as_str));
        let out = git_output(self.repo(), &args, None, env)?;
        let text = String::from_utf8_lossy(&out.stdout);
        let pushed: Vec<Pushed> = text.lines().filter_map(porcelain).collect();
        if pushed.len() != names.len() {
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

pub(crate) fn trimmed(out: Vec<u8>) -> Result<String, StoreError> {
    String::from_utf8(out)
        .map(|s| s.trim().to_owned())
        .map_err(|_| StoreError::Output {
            what: "git printed bytes that are not UTF-8".into(),
        })
}
