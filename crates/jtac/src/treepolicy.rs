//! The `.jtac/policy.toml` of a tree, as coverage asks of it (DESIGN.md section 15.2,
//! decisions 57 and 60): a root's `unread` and `policy` are its signer's word, so a reader
//! asks the tree. The entry is read by `repo::entry_at`, the call the gate loads a policy
//! by, so git resolves the path for both and a reader never reads "no `ci` check" off a
//! tree the gate would load one from. Nothing is fetched: an object the clone lacks is
//! `NotHeld`, by the id a fetch must name.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use jtac_checks::git::{GitError, git};
use jtac_core::GitOid;
use jtac_gate::PolicyKind;
use jtac_policy::POLICY_PATH;
use jtac_store::{StoreError, git_objects};

use crate::refuse::Refusal;
use crate::repo::{self, Entry, RepoError};

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

/// The policy of each of `trees`: one entry read per tree, then each distinct file once.
pub fn read(
    repo: &Path,
    trees: &BTreeSet<GitOid>,
) -> Result<BTreeMap<GitOid, TreePolicy>, Refusal> {
    let mut entries = Vec::new();
    for tree in trees {
        entries.push(match repo::entry_at(repo, &tree.hex(), POLICY_PATH) {
            Ok(entry) => Ok(entry),
            Err(RepoError::Git(GitError::Failed { .. })) => Err(lacking(repo, tree)),
            Err(e) => return Err(Refusal::repository(&e)),
        });
    }
    let files: BTreeSet<&String> = (entries.iter())
        .filter_map(|e| match e {
            Ok(Entry::File(blob)) => Some(blob),
            _ => None,
        })
        .collect();
    let files: Vec<String> = files.into_iter().cloned().collect();
    let read = git_objects(repo, &files).map_err(|e| match e {
        StoreError::Git(e) => Refusal::repository(&RepoError::Git(e)),
        other => Refusal::internal(other),
    })?;
    let held: BTreeMap<&String, Option<Vec<u8>>> = files.iter().zip(read).collect();
    Ok((trees.iter().zip(&entries))
        .map(|(tree, entry)| {
            let policy = match entry {
                Ok(Entry::None) => TreePolicy::None,
                Ok(Entry::Other) => TreePolicy::Unreadable,
                Ok(Entry::File(blob)) => match &held[blob] {
                    Some(bytes) => parse(bytes),
                    None => TreePolicy::NotHeld(blob.clone()),
                },
                Err(lacking) => TreePolicy::NotHeld(lacking.clone()),
            };
            (*tree, policy)
        })
        .collect())
}

/// The object git could not read on the way to the entry: the directory that holds the
/// policy when the tree itself is here, else the tree.
fn lacking(repo: &Path, tree: &GitOid) -> String {
    let dir = POLICY_PATH.split('/').next().unwrap_or(POLICY_PATH);
    let listed = git(repo, &["ls-tree", &tree.hex(), "--", dir]).unwrap_or_default();
    let listed = String::from_utf8_lossy(&listed);
    match listed.split(['\t', ' ']).collect::<Vec<_>>()[..] {
        [_, "tree", id, _] => id.to_owned(),
        _ => tree.hex(),
    }
}

/// The remote a partial clone fetches missing objects from, for the refusal's command;
/// `None` in a full clone. It decides no reading.
pub fn promisor(repo: &Path) -> Result<Option<String>, Refusal> {
    let args = [
        "config",
        "--type=bool",
        "--get-regexp",
        r"^remote\..*\.promisor$",
    ];
    let marked = repo::probe(repo, &args, 1).map_err(|e| Refusal::repository(&e))?;
    let name = |line: &str| {
        let key = line.strip_suffix(" true")?;
        Some(
            key.strip_prefix("remote.")?
                .strip_suffix(".promisor")?
                .to_owned(),
        )
    };
    Ok(marked.as_deref().and_then(|out| out.lines().find_map(name)))
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

    /// A tree built from index entries, so no platform has to make a symlink.
    fn tree_of(repo: &jtac_testkit::Repo, entries: &[(&str, &str, &str)]) -> GitOid {
        repo.git(&["read-tree", "--empty"]);
        for (mode, id, path) in entries {
            let info = format!("{mode},{id},{path}");
            repo.git(&["update-index", "--add", "--cacheinfo", &info]);
        }
        format!("sha1:{}", repo.git(&["write-tree"]))
            .parse()
            .unwrap()
    }

    /// The gate loads a policy only from a regular file (`repo::blob_at`); the reader
    /// asks the same entry, so a link whose text parses as a policy, a gitlink and a
    /// directory at the path are unreadable, never "no `ci` check".
    #[test]
    fn the_policy_of_a_tree_is_what_its_entry_holds_and_only_a_regular_file_is_parsed() {
        let repo = jtac_testkit::Repo::init(&[]);
        let blob = |name: &str, text: &str| {
            repo.write(name, text.as_bytes());
            repo.git(&["hash-object", "-w", name])
        };
        let plain = blob("plain", "version = 1\n");
        let ci = blob(
            "ci",
            "version = 2\n[protected]\npaths = [\".github/**\"]\n[checks.test]\nci = { workflow = \".github/workflows/ci.yml\" }\n",
        );
        repo.write("f", b"f\n");
        let commit = repo.commit("one");
        let path = POLICY_PATH;
        let cases = [
            (
                "a regular file",
                vec![("100644", plain.as_str(), path)],
                TreePolicy::Read(vec![]),
            ),
            (
                "an executable one",
                vec![("100755", plain.as_str(), path)],
                TreePolicy::Read(vec![]),
            ),
            (
                "one with a ci check",
                vec![("100644", ci.as_str(), path)],
                TreePolicy::Read(vec!["test".into()]),
            ),
            (
                "no .jtac",
                vec![("100644", plain.as_str(), "src/policy.toml")],
                TreePolicy::None,
            ),
            (
                "a .jtac without the file",
                vec![("100644", plain.as_str(), ".jtac/signers")],
                TreePolicy::None,
            ),
            (
                "a file named .jtac",
                vec![("100644", plain.as_str(), ".jtac")],
                TreePolicy::None,
            ),
            (
                "a link whose text is a policy",
                vec![("120000", plain.as_str(), path)],
                TreePolicy::Unreadable,
            ),
            (
                "a gitlink",
                vec![("160000", commit.as_str(), path)],
                TreePolicy::Unreadable,
            ),
            (
                "a directory at the path",
                vec![("100644", plain.as_str(), ".jtac/policy.toml/x")],
                TreePolicy::Unreadable,
            ),
        ];
        let trees: Vec<GitOid> = cases.iter().map(|(_, e, _)| tree_of(&repo, e)).collect();
        let absent: GitOid = "sha1:00000000000000000000000000000000000000aa"
            .parse()
            .unwrap();
        let asked: BTreeSet<GitOid> = trees.iter().copied().chain([absent]).collect();
        let got = read(repo.path(), &asked).unwrap();
        for ((name, _, want), tree) in cases.iter().zip(&trees) {
            assert_eq!(&got[tree], want, "{name}");
        }
        assert_eq!(
            got[&absent],
            TreePolicy::NotHeld(absent.hex()),
            "a tree not held"
        );
    }

    /// A tree object written byte for byte, as old tooling and a server without fsck let in.
    fn literal(repo: &jtac_testkit::Repo, mode: &str, name: &str, id: &str) -> GitOid {
        let raw = |hex: &str| -> Vec<u8> {
            (0..hex.len() / 2)
                .map(|i| u8::from_str_radix(&hex[2 * i..2 * i + 2], 16).unwrap())
                .collect()
        };
        let bytes = [mode.as_bytes(), b" ", name.as_bytes(), b"\0", &raw(id)].concat();
        repo.write("raw-tree", &bytes);
        let args = ["hash-object", "-t", "tree", "--literally", "-w", "raw-tree"];
        format!("sha1:{}", repo.git(&args)).parse().unwrap()
    }

    /// git reads a mode by its type bits, not its spelling. A reader that compared the
    /// spelling read "no policy" off a tree the gate loads a `ci` policy from, so the path
    /// is git's to resolve, here as in `repo::blob_at`.
    #[test]
    fn the_policy_of_a_tree_is_what_git_resolves_at_the_path() {
        let repo = jtac_testkit::Repo::init(&[]);
        let text = "version = 2\n[protected]\npaths = [\".github/**\"]\n[checks.test]\nci = { workflow = \".github/workflows/ci.yml\" }\n";
        repo.write("ci", text.as_bytes());
        let ci = repo.git(&["hash-object", "-w", "ci"]);
        let dir = tree_of(&repo, &[("100644", ci.as_str(), "policy.toml")]).hex();
        let loose = literal(&repo, "100664", "policy.toml", &ci).hex();
        let trees = [
            (
                "a zero-padded directory mode",
                literal(&repo, "040000", ".jtac", &dir),
            ),
            (
                "a directory with permission bits",
                literal(&repo, "40755", ".jtac", &dir),
            ),
            (
                "a group-writable file mode",
                literal(&repo, "40000", ".jtac", &loose),
            ),
        ];
        let asked: BTreeSet<GitOid> = trees.iter().map(|(_, t)| *t).collect();
        let got = read(repo.path(), &asked).unwrap();
        for (name, tree) in &trees {
            assert_eq!(got[tree], TreePolicy::Read(vec!["test".into()]), "{name}");
            let head = repo::Head {
                commit: *tree,
                tree: *tree,
            };
            let loaded = repo::blob_at(repo.path(), &head, POLICY_PATH).unwrap();
            assert_eq!(
                loaded.map(|(id, _)| id),
                Some(ci.clone()),
                "{name}: the gate"
            );
        }
    }
}
