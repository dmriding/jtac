//! The `.jtac/policy.toml` of a tree, as coverage asks of it (DESIGN.md section 15.2,
//! decisions 57 and 60): a root's `unread` and `policy` are its signer's word, so a reader
//! asks the tree. Read from git and never fetched, and classified by the tree's own entry,
//! the rule `repo::blob_at` loads a policy by: a regular file is parsed, anything else at
//! the path is unreadable, and only a tree that is here and holds no entry has no policy.
//! git's answer for a path, `missing`, does not tell a path a tree lacks from an object the
//! clone does not hold, so the trees are read as objects and no path is asked.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use jtac_core::GitOid;
use jtac_gate::PolicyKind;
use jtac_policy::POLICY_PATH;
use jtac_store::{StoreError, git_objects};

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

/// One step down the policy path: what the tree says, before the next object is read.
enum Step {
    Is(TreePolicy),
    Next(String),
}

/// The policy of each of `trees`: three reads, the trees, the directories that hold the
/// file, the files, each distinct object once. `format` is the repository's hash.
pub fn read(
    repo: &Path,
    format: &str,
    trees: &BTreeSet<GitOid>,
) -> Result<BTreeMap<GitOid, TreePolicy>, Refusal> {
    let width = if format == "sha256" { 32 } else { 20 };
    let (dir, file) = POLICY_PATH.split_once('/').unwrap_or(("", POLICY_PATH));
    let tops: Vec<String> = trees.iter().map(GitOid::hex).collect();
    let top = level(repo, &tops, |tree| match entry(tree, dir, width) {
        Ok(Some((mode, id))) if mode == "40000" => Step::Next(id),
        Ok(_) => Step::Is(TreePolicy::None),
        Err(Malformed) => Step::Is(TreePolicy::Unreadable),
    })?;
    let dirs = level(repo, &next(&top), |tree| match entry(tree, file, width) {
        Ok(Some((mode, id))) if mode == "100644" || mode == "100755" => Step::Next(id),
        Ok(None) => Step::Is(TreePolicy::None),
        Ok(Some(_)) | Err(Malformed) => Step::Is(TreePolicy::Unreadable),
    })?;
    let files = level(repo, &next(&dirs), |blob| Step::Is(parse(blob)))?;
    Ok((trees.iter().zip(&tops))
        .map(|(tree, hex)| (*tree, follow(&top[hex], &[&dirs, &files])))
        .collect())
}

/// Walk a tree's steps down the levels below it to what the path holds.
fn follow<'a>(mut step: &'a Step, below: &[&'a BTreeMap<String, Step>]) -> TreePolicy {
    for level in below {
        match step {
            Step::Is(policy) => return policy.clone(),
            Step::Next(id) => step = &level[id],
        }
    }
    match step {
        Step::Is(policy) => policy.clone(),
        Step::Next(_) => TreePolicy::Unreadable,
    }
}

/// The ids the level above points at, each once.
fn next(level: &BTreeMap<String, Step>) -> Vec<String> {
    let ids: BTreeSet<&String> = (level.values())
        .filter_map(|s| match s {
            Step::Next(id) => Some(id),
            Step::Is(_) => None,
        })
        .collect();
    ids.into_iter().cloned().collect()
}

/// Read `ids` as objects and take one step in each; an object this clone lacks is
/// `NotHeld` by its own id, the one a fetch must name.
fn level(
    repo: &Path,
    ids: &[String],
    step: impl Fn(&[u8]) -> Step,
) -> Result<BTreeMap<String, Step>, Refusal> {
    let read = git_objects(repo, ids).map_err(|e| match e {
        StoreError::Git(e) => Refusal::repository(&RepoError::Git(e)),
        other => Refusal::internal(other),
    })?;
    Ok((ids.iter().zip(read))
        .map(|(id, object)| {
            let taken = match object {
                Some(bytes) => step(&bytes),
                None => Step::Is(TreePolicy::NotHeld(id.clone())),
            };
            (id.clone(), taken)
        })
        .collect())
}

/// A tree object that is not `<mode> <name>\0<id>` repeated.
struct Malformed;

/// The entry `name` of a raw tree object as (mode, id in hex); `width` is the id's bytes.
fn entry(tree: &[u8], name: &str, width: usize) -> Result<Option<(String, String)>, Malformed> {
    let mut rest = tree;
    while !rest.is_empty() {
        let nul = rest.iter().position(|b| *b == 0).ok_or(Malformed)?;
        let (head, tail) = (&rest[..nul], &rest[nul + 1..]);
        let space = head.iter().position(|b| *b == b' ').ok_or(Malformed)?;
        let id = tail.get(..width).ok_or(Malformed)?;
        if &head[space + 1..] == name.as_bytes() {
            let mode = String::from_utf8_lossy(&head[..space]).into_owned();
            return Ok(Some((
                mode,
                id.iter().map(|b| format!("{b:02x}")).collect(),
            )));
        }
        rest = &tail[width..];
    }
    Ok(None)
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
    /// classifies by the same entry, so a link whose text parses as a policy, a gitlink
    /// and a directory at the path are unreadable, never "no `ci` check".
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
        let got = read(repo.path(), "sha1", &asked).unwrap();
        for ((name, _, want), tree) in cases.iter().zip(&trees) {
            assert_eq!(&got[tree], want, "{name}");
        }
        assert_eq!(
            got[&absent],
            TreePolicy::NotHeld(absent.hex()),
            "a tree not held"
        );
    }

    #[test]
    fn a_raw_tree_entry_is_found_by_its_whole_name_and_a_cut_tree_is_malformed() {
        let mut tree = b"100644 a\0".to_vec();
        tree.extend([0x11; 20]);
        tree.extend(b"40000 .jtac\0");
        tree.extend([0xab; 20]);
        let found = entry(&tree, ".jtac", 20).ok().flatten().unwrap();
        assert_eq!(found, ("40000".to_owned(), "ab".repeat(20)));
        assert!(entry(&tree, "jtac", 20).ok().flatten().is_none());
        assert!(entry(&tree[..tree.len() - 1], ".jtac", 20).is_err());
        assert!(entry(b"100644", "a", 20).is_err());
    }
}
