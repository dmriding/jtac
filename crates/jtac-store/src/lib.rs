//! Local content-addressed store and the git ref layer (DESIGN.md section 9.6).
//!
//! Impure: gathers facts and decides nothing. Every read re-hashes what it finds, so a file
//! that is not what its name says is an error, never a document.

mod batch;
mod cache;
mod fs;
mod levers;
mod objects;
mod refs;
mod roots;
mod state;

use std::path::{Path, PathBuf};

use jtac_checks::git::{GitError, git};
use jtac_core::{CanonicalError, ReadError};

pub use batch::Read;
pub use cache::FileCache;
pub use refs::{Listed, PushOutcome, Pushed, Roots, ref_name};
pub use roots::StoredRoot;

/// One clone's store: `<git common dir>/jtac/`, shared by every linked worktree.
#[derive(Clone, Debug)]
pub struct Store {
    repo: PathBuf,
    dir: PathBuf,
}

impl Store {
    /// The store of the repository whose work tree root is `repo`. Creates nothing; the first
    /// write does.
    pub fn open(repo: &Path) -> Result<Self, StoreError> {
        let out = git(
            repo,
            &["rev-parse", "--path-format=absolute", "--git-common-dir"],
        )?;
        let text = String::from_utf8(out).map_err(|_| StoreError::Output {
            what: "git common dir is not UTF-8".into(),
        })?;
        Ok(Self {
            repo: repo.to_path_buf(),
            dir: Path::new(text.trim_end_matches(['\n', '\r'])).join("jtac"),
        })
    }

    /// The store directory.
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// The file-backed check cache under `cache/`.
    pub fn cache(&self) -> FileCache {
        FileCache::new(self.dir.join("cache"))
    }

    pub(crate) fn repo(&self) -> &Path {
        &self.repo
    }
}

/// Why the store could not read or write. Library errors; the CLI writes the three parts.
#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    /// A git call that could not answer.
    #[error(transparent)]
    Git(#[from] GitError),
    /// A file that could not be read or written.
    #[error("{path}: {source}")]
    Io {
        /// The file.
        path: PathBuf,
        /// The OS error.
        source: std::io::Error,
    },
    /// What the caller handed over has no canonical form.
    #[error(transparent)]
    Canonical(#[from] CanonicalError),
    /// A stored file that is not what its name says.
    #[error("{path}: {what}")]
    Corrupt {
        /// The file.
        path: PathBuf,
        /// What was wrong.
        what: String,
    },
    /// A ref under `refs/jtac/` whose content is not a root for its commit.
    #[error("{name}: {what}")]
    Ref {
        /// The ref.
        name: String,
        /// What was wrong.
        what: String,
    },
    /// The commit already has a root ref: another run wrote it first (DESIGN.md section
    /// 11.1 step 6). Nothing was written.
    #[error("{name} already holds a root")]
    Exists {
        /// The ref.
        name: String,
    },
    /// A root the caller handed over that would not read back, refused before anything is
    /// written.
    #[error("the root is not storable: {what}")]
    Invalid {
        /// What was wrong.
        what: String,
    },
    /// Git printed something this reader could not follow.
    #[error("unreadable git output: {what}")]
    Output {
        /// What was expected.
        what: String,
    },
}

impl StoreError {
    pub(crate) fn io(path: &Path) -> impl FnOnce(std::io::Error) -> Self {
        let path = path.to_path_buf();
        move |source| Self::Io { path, source }
    }

    pub(crate) fn corrupt(path: &Path, what: impl ToString) -> Self {
        Self::Corrupt {
            path: path.to_path_buf(),
            what: what.to_string(),
        }
    }

    pub(crate) fn unreadable(path: &Path, e: ReadError) -> Self {
        Self::corrupt(path, e)
    }
}
