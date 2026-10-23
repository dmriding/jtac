//! The head's tree on disk for its checks (DESIGN.md section 15, decision 47): the repository
//! itself when the tip's tree is the head's, a squash or a rebase that changed nothing, else a
//! detached worktree in a directory of its own under the system temp dir, never inside the
//! clone, since a tool that searches upward (cargo, node) would otherwise read the merge
//! tree's configuration as the head's. The directory is created private and fresh, with a
//! name nobody can guess, since a shared temp dir lets another user plant a path git would
//! fill. Removed when the comparison is done; a registration a dead run left behind is pruned
//! first, so a crash never blocks the next job.

use std::hash::{BuildHasher, Hasher};
use std::path::{Path, PathBuf};

use jtac_checks::diff::git::git;

use crate::repo::Head;

/// Where the head's checks run.
pub struct HeadTree {
    repo: PathBuf,
    path: PathBuf,
    /// The private directory holding `path`, when one was made.
    dir: Option<PathBuf>,
}

impl HeadTree {
    /// The repository itself: the head has no root to compare, or its tree is the tip's.
    pub fn here(repo: &Path) -> Self {
        Self {
            repo: repo.to_path_buf(),
            path: repo.to_path_buf(),
            dir: None,
        }
    }

    /// Check `head` out when its tree is not `tip`'s. An error is the text of what failed,
    /// the caller's refusal.
    pub fn checkout(repo: &Path, tip: &Head, head: &Head) -> Result<Self, String> {
        if tip.tree == head.tree {
            return Ok(Self::here(repo));
        }
        let dir =
            private_dir().map_err(|e| format!("no private directory under the temp dir: {e}"))?;
        let path = dir.join("head");
        let shown = path.to_string_lossy().into_owned();
        let _ = git(repo, &["worktree", "prune"]);
        let added = git(
            repo,
            &[
                "worktree",
                "add",
                "--detach",
                "--quiet",
                &shown,
                &head.commit.hex(),
            ],
        );
        if let Err(e) = added {
            let _ = std::fs::remove_dir_all(&dir);
            return Err(e.to_string());
        }
        Ok(Self {
            repo: repo.to_path_buf(),
            path,
            dir: Some(dir),
        })
    }

    /// The directory the checks run in.
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for HeadTree {
    fn drop(&mut self) {
        if let Some(dir) = &self.dir {
            let shown = self.path.to_string_lossy().into_owned();
            let _ = git(&self.repo, &["worktree", "remove", "--force", &shown]);
            let _ = std::fs::remove_dir_all(dir);
        }
    }
}

/// A fresh directory of this process's own under the temp dir: created, never adopted, so a
/// path planted there by anyone else fails the create and the next name is tried; readable by
/// its owner alone where the platform says so.
fn private_dir() -> std::io::Result<PathBuf> {
    let temp = std::env::temp_dir();
    let mut last = None;
    for _ in 0..16 {
        let mut h = std::collections::hash_map::RandomState::new().build_hasher();
        h.write_u32(std::process::id());
        let dir = temp.join(format!("jtac-{:016x}", h.finish()));
        match create_private(&dir) {
            Ok(()) => return Ok(dir),
            Err(e) => last = Some(e),
        }
    }
    Err(last.unwrap_or_else(|| std::io::Error::other("no name was free")))
}

#[cfg(unix)]
fn create_private(dir: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::DirBuilderExt;
    std::fs::DirBuilder::new().mode(0o700).create(dir)
}

#[cfg(not(unix))]
fn create_private(dir: &Path) -> std::io::Result<()> {
    std::fs::create_dir(dir)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::repo;
    use jtac_testkit::Repo;

    /// Round one's finding: a worktree inside the clone let upward-searching tools read the
    /// merge tree's files; round two's: a guessable path in a shared temp dir can be planted.
    #[test]
    fn the_head_checks_out_in_a_fresh_private_directory_outside_the_clone_and_leaves_nothing() {
        let r = Repo::init(&[]);
        r.write("a.txt", b"a\n");
        r.commit("a");
        let format = repo::format(r.path()).unwrap();
        let old = repo::head(r.path()).unwrap();
        r.write("b.txt", b"b\n");
        r.commit("b");
        let tip = repo::head(r.path()).unwrap();
        let same = HeadTree::checkout(r.path(), &tip, &tip).unwrap();
        assert_eq!(same.path(), r.path());
        let canon = std::fs::canonicalize(r.path()).unwrap();
        let tree = HeadTree::checkout(r.path(), &tip, &old).unwrap();
        let path = tree.path().to_path_buf();
        assert!(path.join("a.txt").exists() && !path.join("b.txt").exists());
        assert!(!std::fs::canonicalize(&path).unwrap().starts_with(&canon));
        let dir = path.parent().unwrap().to_path_buf();
        assert!(dir.starts_with(std::env::temp_dir()));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&dir).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o700);
        }
        let head = repo::peel(r.path(), &format, "HEAD").unwrap();
        assert_eq!(head, tip, "the clone's own checkout is untouched");
        drop(tree);
        assert!(!dir.exists(), "the directory is gone");
        let list = r.git(&["worktree", "list", "--porcelain"]);
        assert_eq!(list.matches("worktree ").count(), 1, "{list}");
    }

    #[test]
    fn a_planted_path_is_never_adopted() {
        let a = private_dir().unwrap();
        let b = private_dir().unwrap();
        assert_ne!(a, b);
        assert!(
            create_private(&a).is_err(),
            "an existing path fails the create"
        );
        std::fs::remove_dir(&a).unwrap();
        std::fs::remove_dir(&b).unwrap();
    }
}
