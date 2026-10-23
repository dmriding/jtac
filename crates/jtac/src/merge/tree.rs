//! The head's tree on disk for its checks (DESIGN.md section 15, decision 47): the repository
//! itself when the tip's tree is the head's, a squash or a rebase that changed nothing, else a
//! detached worktree in a directory of its own under the system temp dir, never inside the
//! clone, since a tool that searches upward (cargo, node) would otherwise read the merge
//! tree's configuration as the head's. Removed when the comparison is done; a registration a
//! dead run left behind is pruned first, so a crash never blocks the next job.

use std::path::{Path, PathBuf};

use jtac_checks::diff::git::git;

use crate::repo::Head;

/// Where the head's checks run.
pub struct HeadTree {
    repo: PathBuf,
    path: PathBuf,
    added: bool,
}

impl HeadTree {
    /// The repository itself: the head has no root to compare, or its tree is the tip's.
    pub fn here(repo: &Path) -> Self {
        Self {
            repo: repo.to_path_buf(),
            path: repo.to_path_buf(),
            added: false,
        }
    }

    /// Check `head` out when its tree is not `tip`'s. An error is git's text, the caller's
    /// refusal.
    pub fn checkout(repo: &Path, tip: &Head, head: &Head) -> Result<Self, String> {
        if tip.tree == head.tree {
            return Ok(Self::here(repo));
        }
        let hex = head.commit.hex();
        let path =
            std::env::temp_dir().join(format!("jtac-head-{}-{}", &hex[..12], std::process::id()));
        let shown = path.to_string_lossy().into_owned();
        let _ = git(repo, &["worktree", "prune"]);
        git(
            repo,
            &["worktree", "add", "--detach", "--quiet", &shown, &hex],
        )
        .map_err(|e| e.to_string())?;
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
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }
}
