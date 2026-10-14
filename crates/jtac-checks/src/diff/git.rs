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

/// Run `git` with `args` in `repo` and return its stdout. A non-zero exit is an error
/// carrying the first stderr line; the caller that expects one matches on `Failed`.
pub fn git(repo: &Path, args: &[&str]) -> Result<Vec<u8>, GitError> {
    git_with(repo, args, None, &[])
}

/// [`git`] with bytes on stdin (closed when `None`) and `env` on top of the inherited one.
pub fn git_with(
    repo: &Path,
    args: &[&str],
    stdin: Option<&[u8]>,
    env: &[(&str, &str)],
) -> Result<Vec<u8>, GitError> {
    let out = git_output(repo, args, stdin, env)?;
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        return Err(GitError::Failed {
            argv: argv(args),
            status: out.status.code(),
            stderr: stderr.lines().next().unwrap_or_default().to_owned(),
        });
    }
    Ok(out.stdout)
}

/// [`git_with`] returning the whole output whatever the exit, for a caller that reads
/// stdout on failure too (`push --porcelain`). Only a spawn failure is an error. Stdin is
/// fed from a thread while the output drains, so a child that fills stdout before it reads
/// its input (`cat-file --batch`) cannot deadlock; a child that exits early is its own
/// answer, not a broken pipe. Replace refs are never followed, the commit graph is never
/// read, and a partial clone never fetches an object on demand: a `git replace --graft` or an
/// edited graph file would show a history the root does not name, and a reader of evidence
/// must not touch the network (`a_replace_graft_does_not_hide_the_change`). `GIT_CONFIG` is
/// dropped: it steers `git config` alone, and a config read must see what `git push` sees
/// (`a_mirror_push_is_refused_because_git_hides_its_deletions`).
pub fn git_output(
    repo: &Path,
    args: &[&str],
    stdin: Option<&[u8]>,
    env: &[(&str, &str)],
) -> Result<std::process::Output, GitError> {
    use std::io::Write;
    use std::process::Stdio;
    let mut cmd = Command::new("git");
    cmd.args(["-c", "core.commitGraph=false"])
        .args(args)
        .current_dir(repo)
        .env_remove("GIT_CONFIG")
        .env("GIT_NO_REPLACE_OBJECTS", "1")
        .env("GIT_NO_LAZY_FETCH", "1")
        .envs(env.iter().copied())
        .stdin(if stdin.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let spawn = |source| GitError::Spawn {
        argv: argv(args),
        source,
    };
    let mut child = cmd.spawn().map_err(spawn)?;
    let feeder = stdin.map(|bytes| {
        let mut pipe = child.stdin.take().expect("stdin was piped");
        let bytes = bytes.to_vec();
        std::thread::spawn(move || pipe.write_all(&bytes))
    });
    let out = child.wait_with_output().map_err(spawn)?;
    if let Some(feeder) = feeder
        && let Err(e) = feeder.join().expect("the feeder thread does not panic")
        && e.kind() != std::io::ErrorKind::BrokenPipe
    {
        return Err(spawn(e));
    }
    Ok(out)
}

/// `word` as one `sh` argument. A ref name or path git accepts can hold `$(`, `;`, `|` or a
/// quote, and a message that names a command to run must not hand them to the shell
/// (`roots_skips_a_ref_the_writer_would_not_write_with_a_note`).
pub fn quoted(word: &str) -> String {
    format!("'{}'", word.replace('\'', "'\\''"))
}

fn argv(args: &[&str]) -> Vec<String> {
    std::iter::once("git")
        .chain(args.iter().copied())
        .map(str::to_owned)
        .collect()
}
