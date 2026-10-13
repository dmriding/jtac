//! Gather a change set from git: `diff --raw` names each path and blob, `cat-file` reads the
//! blobs. Impure, argv arrays only (DESIGN.md section 8); which base to diff against is the
//! caller's (section 10.3).

use std::path::Path;
use std::process::Command;

use super::change::{Blob, Change};

/// A git call that could not answer. Every variant refuses; nothing falls back to an empty
/// change set, because a diff that silently found nothing would pass every rule.
#[derive(Debug, thiserror::Error)]
pub enum GitError {
    /// `git` did not start.
    #[error("could not run `{}`: {source}", argv.join(" "))]
    Spawn {
        /// The call.
        argv: Vec<String>,
        /// The OS error.
        source: std::io::Error,
    },
    /// `git` exited non-zero.
    #[error("`{}` exited {}: {stderr}", argv.join(" "), status.map_or("by signal".to_owned(), |s| s.to_string()))]
    Failed {
        /// The call.
        argv: Vec<String>,
        /// Exit code, `None` on a signal.
        status: Option<i32>,
        /// Its stderr, first line.
        stderr: String,
    },
    /// `diff --raw -z` output this reader could not follow.
    #[error("unreadable `git diff --raw` entry: {entry}")]
    Raw {
        /// The entry.
        entry: String,
    },
}

const SUBMODULE: &str = "160000";

/// Every file that differs between `base` and `tip` in `repo`, renames paired. A submodule side
/// is absent, so a path that became or replaced a gitlink reads as deleted or added; an absent
/// side is all zeroes in either object format.
pub fn changes(repo: &Path, base: &str, tip: &str) -> Result<Vec<Change>, GitError> {
    let raw = git(
        repo,
        &[
            "diff",
            "--raw",
            "-M",
            "-z",
            "--no-abbrev",
            "--no-color",
            "--end-of-options",
            base,
            tip,
            "--",
        ],
    )?;
    let mut fields = raw
        .split(|&b| b == 0)
        .map(|f| String::from_utf8_lossy(f).into_owned());
    let mut changes = Vec::new();
    while let Some(entry) = fields.next() {
        if entry.is_empty() {
            continue;
        }
        let parts: Vec<&str> = entry
            .strip_prefix(':')
            .unwrap_or_default()
            .split(' ')
            .collect();
        let [old_mode, new_mode, old_sha, new_sha, status] = parts[..] else {
            return Err(GitError::Raw { entry });
        };
        let renamed = status.starts_with('R');
        let first = fields.next().ok_or_else(|| GitError::Raw {
            entry: entry.clone(),
        })?;
        let (from, path) = if renamed {
            let to = fields.next().ok_or_else(|| GitError::Raw {
                entry: entry.clone(),
            })?;
            (Some(first), to)
        } else {
            (None, first)
        };
        let blob = |mode: &str, sha: &str| -> Result<Option<Blob>, GitError> {
            if sha.bytes().all(|b| b == b'0') || mode == SUBMODULE {
                return Ok(None);
            }
            Ok(Some(Blob::of(&git(repo, &["cat-file", "blob", sha])?)))
        };
        changes.push(Change {
            path,
            from,
            old: blob(old_mode, old_sha)?,
            new: blob(new_mode, new_sha)?,
        });
    }
    Ok(changes)
}

/// The merge base of `a` and `b`.
pub fn merge_base(repo: &Path, a: &str, b: &str) -> Result<String, GitError> {
    let out = git(repo, &["merge-base", "--end-of-options", a, b])?;
    Ok(String::from_utf8_lossy(&out).trim().to_owned())
}

fn git(repo: &Path, args: &[&str]) -> Result<Vec<u8>, GitError> {
    let argv = || {
        std::iter::once("git")
            .chain(args.iter().copied())
            .map(str::to_owned)
            .collect()
    };
    let out = Command::new("git")
        .args(args)
        .current_dir(repo)
        .stdin(std::process::Stdio::null())
        .output()
        .map_err(|source| GitError::Spawn {
            argv: argv(),
            source,
        })?;
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        return Err(GitError::Failed {
            argv: argv(),
            status: out.status.code(),
            stderr: stderr.lines().next().unwrap_or_default().to_owned(),
        });
    }
    Ok(out.stdout)
}
