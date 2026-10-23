//! The head's tree on disk for its checks (DESIGN.md section 15): the repository itself
//! when the tip's tree is the head's, a squash or a rebase that changed nothing, else a
//! detached worktree under the store, removed when the comparison is done. A stale one from
//! a run that died is removed first, so a crash never blocks the next job.

use std::path::{Path, PathBuf};

use jtac_checks::diff::git::git;
use jtac_store::Store;

use crate::refuse::Refusal;
use crate::repo::Head;

/// Where the head's checks run.
pub struct HeadTree {
    repo: PathBuf,
    path: PathBuf,
    added: bool,
}

impl HeadTree {
    /// Check `head` out when its tree is not `tip`'s.
    pub fn checkout(repo: &Path, store: &Store, tip: &Head, head: &Head) -> Result<Self, Refusal> {
        if tip.tree == head.tree {
            return Ok(Self {
                repo: repo.to_path_buf(),
                path: repo.to_path_buf(),
                added: false,
            });
        }
        let path = store.dir().join("heads").join(head.commit.hex());
        let shown = path.to_string_lossy().into_owned();
        if path.exists() {
            let _ = git(repo, &["worktree", "remove", "--force", &shown]);
            let _ = git(repo, &["worktree", "prune"]);
        }
        git(
            repo,
            &[
                "worktree",
                "add",
                "--detach",
                "--quiet",
                &shown,
                &head.commit.hex(),
            ],
        )
        .map_err(|e| Refusal::worktree(&e.to_string()))?;
        Ok(Self {
            repo: repo.to_path_buf(),
            path,
            added: true,
        })
    }

    /// The directory the checks run in.
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for HeadTree {
    fn drop(&mut self) {
        if self.added {
            let shown = self.path.to_string_lossy().into_owned();
            let _ = git(&self.repo, &["worktree", "remove", "--force", &shown]);
        }
    }
}
