//! The `.jtac/policy.toml` of a tree, as coverage asks of it (DESIGN.md section 15.2,
//! decisions 57 and 60): a root's `unread` and `policy` are its signer's word, so a reader
//! asks the tree. The entry is read by `repo::entry_at` and its bytes by `repo::held`, the
//! calls the gate loads a policy by, so git resolves the path and names the object for both
//! and a reader never reads "no `ci` check" off a tree the gate would load one from.
//! Nothing is fetched: an object the clone lacks is `NotHeld`, by the id a fetch must name,
//! and only where git says that object is missing. The read is total: a root is judged
//! after it, so no tree a root names may stop the command, and one git cannot walk is
//! `Unreadable`.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use jtac_checks::git::{GitError, git};
use jtac_core::GitOid;
use jtac_gate::PolicyKind;
use jtac_policy::POLICY_PATH;

use crate::refuse::Refusal;
use crate::repo::{self, Entry, Held, RepoError};

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

    /// Why a root without `from` that names this tree is invalid (decision 57): its
    /// `unread` is not this file's `ci` checks. `None` too where nothing shows which
    /// checks it left out: that root is partial whatever it lists, or not held. The one
    /// statement of the rule, for the ledger, `replay` and the writer.
    pub fn refuses(&self, unread: &[String]) -> Option<String> {
        let ci = self.ci_checks().filter(|ci| *ci != unread)?;
        let list = |ids: &[String]| match ids {
            [] => "no check".to_owned(),
            ids => format!("`{}`", ids.join("`, `")),
        };
        Some(format!(
            "it lists {} unread, and the policy in the tree it names leaves {} to CI",
            list(unread),
            list(ci)
        ))
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
            Err(RepoError::Git(GitError::Failed { .. })) => Err(lacking(repo, tree)?),
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
    let read = repo::held(repo, &files).map_err(|e| Refusal::repository(&e))?;
    let held: BTreeMap<&String, Held> = files.iter().zip(read).collect();
    Ok((trees.iter().zip(&entries))
        .map(|(tree, entry)| {
            let policy = match entry {
                Ok(Entry::None) => TreePolicy::None,
                Ok(Entry::Other) => TreePolicy::Unreadable,
                Ok(Entry::File(blob)) => match &held[blob] {
                    Held::Blob(bytes) => parse(bytes),
                    Held::NotBlob => TreePolicy::Unreadable,
                    Held::Missing => TreePolicy::NotHeld(blob.clone()),
                },
                Err(Some(lacking)) => TreePolicy::NotHeld(lacking.clone()),
                Err(None) => TreePolicy::Unreadable,
            };
            (*tree, policy)
        })
        .collect())
}

/// The object this clone lacks on the way to the entry: the directory that holds the
/// policy when the tree itself is here, else the tree. `None` when git holds that object
/// after all: the tree cannot be walked, a root may name any id, and a clone is never told
/// to fetch what it has (`a_tree_git_cannot_walk_is_unreadable_and_stops_no_reader`).
fn lacking(repo: &Path, tree: &GitOid) -> Result<Option<String>, Refusal> {
    let dir = POLICY_PATH.split('/').next().unwrap_or(POLICY_PATH);
    let listed = git(repo, &["ls-tree", &tree.hex(), "--", dir]).unwrap_or_default();
    let listed = String::from_utf8_lossy(&listed);
    let id = match listed.split(['\t', ' ']).collect::<Vec<_>>()[..] {
        [_, "tree", id, _] => id.to_owned(),
        _ => tree.hex(),
    };
    let held = repo::held(repo, std::slice::from_ref(&id)).map_err(|e| Refusal::repository(&e))?;
    Ok((held == [Held::Missing]).then_some(id))
}

/// The one not-held answer (decision 60): refuse, naming the fetch, when any of `policies`
/// is an object this clone lacks. A command that judges answers 2; a report prints the row.
pub fn held<'a>(
    repo: &Path,
    command: &str,
    policies: impl IntoIterator<Item = &'a TreePolicy>,
) -> Result<(), Refusal> {
    let objects: BTreeSet<&str> = (policies.into_iter())
        .filter_map(|policy| match policy {
            TreePolicy::NotHeld(id) => Some(id.as_str()),
            _ => None,
        })
        .collect();
    if objects.is_empty() {
        return Ok(());
    }
    let objects: Vec<&str> = objects.into_iter().collect();
    let remote = promisor(repo)?;
    Err(Refusal::policy_not_held(
        command,
        &objects,
        remote.as_deref(),
    ))
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

    /// Decision 57's rule in one place: every `ci` check and no other id, in policy order;
    /// no claim where nothing shows which checks were left out.
    #[test]
    fn a_root_is_refused_when_its_unread_is_not_its_trees_ci_checks() {
        let ci = TreePolicy::Read(vec!["acc".into(), "test".into()]);
        let ids = |ids: &[&str]| ids.iter().map(|id| (*id).to_owned()).collect::<Vec<_>>();
        assert_eq!(ci.refuses(&ids(&["acc", "test"])), None);
        assert_eq!(
            ci.refuses(&ids(&["test", "acc"])).as_deref(),
            Some(
                "it lists `test`, `acc` unread, and the policy in the tree it names leaves \
                 `acc`, `test` to CI"
            )
        );
        assert_eq!(
            ci.refuses(&[]).as_deref(),
            Some(
                "it lists no check unread, and the policy in the tree it names leaves `acc`, \
                 `test` to CI"
            )
        );
        for run in [TreePolicy::None, TreePolicy::Read(vec![])] {
            assert_eq!(run.refuses(&[]), None, "{run:?}");
            assert_eq!(
                run.refuses(&ids(&["test"])).as_deref(),
                Some(
                    "it lists `test` unread, and the policy in the tree it names leaves no \
                     check to CI"
                ),
                "{run:?}"
            );
        }
        for unknown in [TreePolicy::Unreadable, TreePolicy::NotHeld("x".into())] {
            assert_eq!(unknown.refuses(&ids(&["test"])), None, "{unknown:?}");
        }
    }

    /// Decision 60's refusal names each object once, with the command, and nothing for a
    /// policy that is here.
    #[test]
    fn the_not_held_answer_names_each_lacking_object_once_and_none_that_is_held() {
        let repo = jtac_testkit::Repo::init(&[]);
        let lacks = |id: &str| TreePolicy::NotHeld(id.to_owned());
        let here = [
            TreePolicy::None,
            TreePolicy::Read(vec![]),
            TreePolicy::Unreadable,
        ];
        assert!(held(repo.path(), "verify", &here).is_ok());
        let asked = [lacks("bbb"), TreePolicy::None, lacks("aaa"), lacks("bbb")];
        let refusal = held(repo.path(), "replay", &asked).unwrap_err();
        assert!(
            refusal.what.contains("does not hold aaa, bbb."),
            "{refusal:?}"
        );
        assert!(
            refusal
                .how
                .starts_with("Run `git fetch origin aaa bbb`, then `jtac replay` again"),
            "{refusal:?}"
        );
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

    /// Issue 168: an entry's mode says file and the object decides. The gate once peeled a
    /// tag of a blob and loaded the policy the reader called unreadable; both ask
    /// `repo::held`, which peels nothing.
    #[test]
    fn a_file_entry_that_names_no_blob_is_a_file_to_neither_the_gate_nor_the_reader() {
        let repo = jtac_testkit::Repo::init(&[]);
        repo.write("p", b"version = 1\n");
        let blob = repo.git(&["hash-object", "-w", "p"]);
        let commit = repo.commit("one");
        repo.git(&["tag", "-a", "-m", "a tag of the file", "policy", &blob]);
        let tag = repo.git(&["rev-parse", "policy"]);
        for (name, id) in [("a tag of a blob", &tag), ("a commit", &commit)] {
            let inner = literal(&repo, "100644", "policy.toml", id).hex();
            let tree = literal(&repo, "40000", ".jtac", &inner);
            let got = read(repo.path(), &BTreeSet::from([tree])).unwrap();
            assert_eq!(got[&tree], TreePolicy::Unreadable, "{name}");
            let head = repo::Head { commit: tree, tree };
            let loaded = repo::blob_at(repo.path(), &head, POLICY_PATH);
            assert!(
                matches!(loaded, Err(RepoError::NotAFile { .. })),
                "{name}: {loaded:?}"
            );
        }
    }

    /// Issue 168: not held is said of an object git reports missing, and of nothing else.
    /// A root may name any id as its tree and is judged after this read, so a tree git
    /// cannot walk, a directory entry that names a blob or a blob named as the tree, is
    /// unreadable, never a fetch and never the command's error.
    #[test]
    fn a_tree_git_cannot_walk_is_unreadable_and_stops_no_reader() {
        let repo = jtac_testkit::Repo::init(&[]);
        repo.write("p", b"version = 1\n");
        let blob = repo.git(&["hash-object", "-w", "p"]);
        let trees = [
            literal(&repo, "40000", ".jtac", &blob),
            format!("sha1:{blob}").parse().unwrap(),
        ];
        let got = read(repo.path(), &trees.into_iter().collect()).unwrap();
        for tree in trees {
            assert_eq!(got[&tree], TreePolicy::Unreadable, "{tree}");
        }
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
