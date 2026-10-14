//! The `pre-push` file `jtac init` installs (DESIGN.md section 11.3, hook rules): `#!/bin/sh`,
//! the version marker, and an `exec` of this binary by absolute path in single quotes. A file
//! jtac did not write is never edited; its owner gets the line to add, and a file that ends
//! with that line is kept. A hook git cannot execute is a gate that is not there, so the mode
//! is judged with the bytes (`only_a_file_jtac_wrote_or_one_that_execs_it_is_kept`).

use std::path::{Path, PathBuf};

use jtac_checks::git::git;

use crate::init::ask;
use crate::refuse::Refusal;

/// Line 2 of every hook file jtac writes.
pub const MARKER: &str = "# jtac-hook v1";

/// What is at the hook's path now.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Found {
    /// Nothing.
    Absent,
    /// A regular file with these bytes, and whether git can run it.
    File {
        /// Its bytes.
        bytes: Vec<u8>,
        /// Any execute bit; always true where there is none.
        executable: bool,
    },
    /// A symlink, a directory, or anything else jtac did not write.
    Other,
}

/// What init does at the hook's path.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Install {
    /// The file is already the one init would write.
    Keep,
    /// Nothing is there.
    Write,
    /// A hook jtac wrote, naming a binary that moved.
    Rewrite,
    /// The right file, not executable.
    Chmod,
}

/// The `exec` line that hands git's arguments and stdin to `jtac hook pre-push`.
pub fn exec_line(exe: &str) -> String {
    format!("exec '{exe}' hook pre-push \"$@\"")
}

/// The whole file for `exe`. A quote in the path would end the shell string early.
pub fn script(exe: &str) -> Result<String, Refusal> {
    if exe.contains('\'') {
        return Err(Refusal::init_hook_path(exe));
    }
    Ok(format!("#!/bin/sh\n{MARKER}\n{}\n", exec_line(exe)))
}

/// Keep, write, rewrite, or chmod; `None` when the file is not one jtac wrote and does not
/// end with the exec line. A file with the marker and anything beyond the three lines was
/// edited by hand and is not rewritten.
pub fn judge(found: &Found, want: &str) -> Option<Install> {
    let ours = |text: &str| {
        let lines: Vec<&str> = text.lines().collect();
        matches!(lines[..], ["#!/bin/sh", marker, exec]
            if (marker == "# jtac-hook" || marker.starts_with("# jtac-hook "))
                && exec.starts_with("exec '")
                && exec.ends_with("' hook pre-push \"$@\""))
    };
    let exec = want.lines().nth(2).unwrap_or_default();
    let ends_with_exec =
        |text: &str| text.lines().rev().find(|l| !l.trim().is_empty()) == Some(exec);
    let (bytes, executable) = match found {
        Found::Absent => return Some(Install::Write),
        Found::Other => return None,
        Found::File { bytes, executable } => (bytes, *executable),
    };
    let text = std::str::from_utf8(bytes).ok()?;
    if bytes == want.as_bytes() || ends_with_exec(text) {
        Some(if executable {
            Install::Keep
        } else {
            Install::Chmod
        })
    } else if ours(text) {
        Some(Install::Rewrite)
    } else {
        None
    }
}

/// The hooks directory git runs from, `core.hooksPath` honoured, as a path from `repo`.
pub fn dir(repo: &Path) -> Result<PathBuf, Refusal> {
    let out = git(repo, &["rev-parse", "--git-path", "hooks"])
        .map_err(|e| Refusal::repository(&e.into()))?;
    Ok(repo.join(String::from_utf8_lossy(&out).trim_end_matches(['\n', '\r'])))
}

/// This binary by absolute path with `/` separators, as `sh` reads it on every platform.
pub fn exe() -> Result<String, Refusal> {
    let exe = std::env::current_exe()
        .and_then(std::fs::canonicalize)
        .map_err(|e| Refusal::exe(&e))?;
    let text = exe
        .to_str()
        .ok_or_else(|| Refusal::init_hook_path(&exe.to_string_lossy()))?;
    let text = match text.strip_prefix(r"\\?\") {
        Some(rest) => rest
            .strip_prefix(r"UNC\")
            .map_or(rest.to_owned(), |r| format!(r"\\{r}")),
        None => text.to_owned(),
    };
    Ok(text.replace('\\', "/"))
}

/// What is at `path`, not following a symlink.
pub fn read(path: &Path) -> Result<Found, Refusal> {
    match path.symlink_metadata() {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Found::Absent),
        Err(e) => Err(Refusal::init_io("the pre-push hook", path, &e)),
        Ok(m) if m.is_file() => std::fs::read(path)
            .map(|bytes| Found::File {
                bytes,
                executable: executable(&m),
            })
            .map_err(|e| Refusal::init_io("the pre-push hook", path, &e)),
        Ok(_) => Ok(Found::Other),
    }
}

#[cfg(unix)]
fn executable(m: &std::fs::Metadata) -> bool {
    use std::os::unix::fs::PermissionsExt;
    m.permissions().mode() & 0o111 != 0
}

#[cfg(not(unix))]
fn executable(_: &std::fs::Metadata) -> bool {
    true
}

/// Write `text` at `path`, executable, creating the hooks directory.
pub fn write(path: &Path, text: &str) -> Result<(), Refusal> {
    let io = |e: std::io::Error| Refusal::init_io("the pre-push hook", path, &e);
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(io)?;
    }
    std::fs::write(path, text).map_err(io)?;
    chmod(path)
}

/// Make `path` executable where a mode exists.
pub fn chmod(path: &Path) -> Result<(), Refusal> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755))
            .map_err(|e| Refusal::init_io("the pre-push hook", path, &e))?;
    }
    let _ = path;
    Ok(())
}

impl Refusal {
    /// A hook file at the path that jtac did not write.
    pub fn init_foreign_hook(path: &Path, exe: &str) -> Self {
        ask(
            format!(
                "`{}` exists and jtac did not write it.",
                path.to_string_lossy()
            ),
            "jtac never edits a hook it did not write: the file is someone's own gate, and \
             rewriting it would drop whatever it enforces. Do not delete it to get past this.",
            format!(
                "Add `{}` as the last line of `{}` (a hook that reads stdin before it leaves \
                 jtac no refs to judge), then run `jtac init` again: a file that ends with \
                 that line is kept.",
                exec_line(exe),
                path.to_string_lossy()
            ),
        )
    }

    /// The binary's path cannot sit inside a single-quoted shell string.
    pub fn init_hook_path(exe: &str) -> Self {
        ask(
            format!("the jtac binary's path `{exe}` holds a single quote or is not UTF-8."),
            "The hook runs jtac by absolute path inside single quotes, so git runs this binary \
             and no other; a quote would end the string and run the rest as shell.",
            "Move the jtac binary to a directory whose path has no quote, then run `jtac \
             init` from there.",
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const EXE: &str = "/workspace/bin/jtac";

    #[test]
    fn the_script_is_the_shebang_the_marker_and_a_quoted_exec() {
        assert_eq!(
            script(EXE).unwrap(),
            "#!/bin/sh\n# jtac-hook v1\nexec '/workspace/bin/jtac' hook pre-push \"$@\"\n"
        );
        let r = script("/workspace/it's/jtac").unwrap_err();
        assert!(r.what.contains("single quote"), "{r:?}");
    }

    #[test]
    fn only_a_file_jtac_wrote_or_one_that_execs_it_is_kept() {
        let want = script(EXE).unwrap();
        let moved = script("/workspace/old/jtac").unwrap();
        let file = |s: &str| Found::File {
            bytes: s.as_bytes().to_vec(),
            executable: true,
        };
        let plain = |s: &str| Found::File {
            bytes: s.as_bytes().to_vec(),
            executable: false,
        };
        let integrated = format!("#!/bin/sh\necho mine\n{}\n", exec_line(EXE));
        let rows = [
            (Found::Absent, Some(Install::Write)),
            (file(&want), Some(Install::Keep)),
            (plain(&want), Some(Install::Chmod)),
            (file(&integrated), Some(Install::Keep)),
            (file(&format!("{integrated}\n\n")), Some(Install::Keep)),
            (plain(&integrated), Some(Install::Chmod)),
            (
                file(&integrated.replace("/workspace/bin", "/workspace/old")),
                None,
            ),
            (file(&format!("{integrated}echo after\n")), None),
            (plain(&moved), Some(Install::Rewrite)),
            (file(&moved), Some(Install::Rewrite)),
            (file(&moved.replace("v1", "v2")), Some(Install::Rewrite)),
            (file(&moved.replace('\n', "\r\n")), Some(Install::Rewrite)),
            (file(&format!("{moved}echo extra\n")), None),
            (file(&moved.replace("# jtac-hook v1", "# mine")), None),
            (
                file(&moved.replace("# jtac-hook v1", "# jtac-hookery")),
                None,
            ),
            (file(&moved.replace("#!/bin/sh", "#!/bin/bash")), None),
            (file(&moved.replace("pre-push", "pre-commit")), None),
            (file("#!/bin/sh\nexit 0\n"), None),
            (
                Found::File {
                    bytes: vec![0xff, 0xfe],
                    executable: true,
                },
                None,
            ),
            (Found::Other, None),
        ];
        for (found, install) in rows {
            assert_eq!(judge(&found, &want), install, "{found:?}");
        }
    }

    #[test]
    fn the_running_binary_is_absolute_with_forward_slashes() {
        let exe = exe().unwrap();
        assert!(!exe.contains('\\'), "{exe}");
        assert!(
            Path::new(&exe).is_absolute() || exe.starts_with("//"),
            "{exe}"
        );
    }
}
