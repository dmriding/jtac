//! The ssh-keygen binary and one way to run it: argv array, bytes on stdin, both streams
//! captured, the inherited environment so the agent is reachable, `TZ=UTC0` so every time
//! it reads or is given means the same instant on every machine, and no askpass program so
//! nothing can open a prompt where there is no terminal.

use std::ffi::OsStr;
use std::io::Write as _;
use std::path::PathBuf;
use std::process::{Command, Output, Stdio};

use crate::SignError;

/// Where ssh-keygen is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Tool {
    program: PathBuf,
}

impl Tool {
    /// `program` is a path or a name looked up on PATH.
    pub fn new(program: impl Into<PathBuf>) -> Self {
        Self {
            program: program.into(),
        }
    }

    /// The binary.
    pub fn program(&self) -> &std::path::Path {
        &self.program
    }

    /// Run and return everything; only a spawn failure is an error.
    pub(crate) fn run(&self, args: &[&OsStr], stdin: &[u8]) -> Result<Output, SignError> {
        let spawn = |source| SignError::Spawn {
            argv: self.argv(args),
            source,
        };
        let mut child = Command::new(&self.program)
            .args(args)
            .env("TZ", "UTC0")
            .env("SSH_ASKPASS_REQUIRE", "never")
            .env_remove("SSH_ASKPASS")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(spawn)?;
        let mut pipe = child.stdin.take().expect("stdin was piped");
        let bytes = stdin.to_vec();
        let feeder = std::thread::spawn(move || pipe.write_all(&bytes));
        let out = child.wait_with_output().map_err(spawn)?;
        if let Err(e) = feeder.join().expect("the feeder thread does not panic")
            && e.kind() != std::io::ErrorKind::BrokenPipe
        {
            return Err(spawn(e));
        }
        Ok(out)
    }

    /// Run and return stdout; a non-zero exit is [`SignError::Failed`].
    pub(crate) fn ok(&self, args: &[&OsStr], stdin: &[u8]) -> Result<Vec<u8>, SignError> {
        let out = self.run(args, stdin)?;
        if !out.status.success() {
            return Err(SignError::Failed {
                argv: self.argv(args),
                stderr: failure(&out),
            });
        }
        Ok(out.stdout)
    }

    pub(crate) fn argv(&self, args: &[&OsStr]) -> Vec<String> {
        std::iter::once(self.program.as_os_str())
            .chain(args.iter().copied())
            .map(|a| a.to_string_lossy().into_owned())
            .collect()
    }
}

/// What a failed `out` said, or its exit status when it said nothing.
pub(crate) fn failure(out: &Output) -> String {
    match first_line(&out.stderr) {
        s if s.is_empty() => format!("no output, {}", out.status),
        s => s,
    }
}

pub(crate) fn first_line(stderr: &[u8]) -> String {
    String::from_utf8_lossy(stderr)
        .lines()
        .find(|l| !l.trim().is_empty())
        .unwrap_or_default()
        .to_owned()
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[test]
    fn the_child_runs_in_utc_without_an_askpass_program() {
        let sh = Tool::new("sh");
        let args: [&OsStr; 2] = [
            "-c".as_ref(),
            "echo \"$TZ|$SSH_ASKPASS_REQUIRE|$SSH_ASKPASS\"; cat".as_ref(),
        ];
        let out = sh.ok(&args, b"fed").unwrap();
        assert_eq!(String::from_utf8_lossy(&out), "UTC0|never|\nfed");
    }

    /// Windows OpenSSH dies before its logger is up when `ProgramData` is unset, so a failure
    /// with nothing on stderr must still say what happened.
    #[test]
    fn a_silent_failure_reports_the_exit_status_and_blank_lines_are_skipped() {
        let sh = Tool::new("sh");
        let silent: [&OsStr; 2] = ["-c".as_ref(), "exit 3".as_ref()];
        let blank: [&OsStr; 2] = [
            "-c".as_ref(),
            "printf '\\n\\n  \\nreal\\n' >&2; exit 1".as_ref(),
        ];
        let what = |args: &[&OsStr]| match sh.ok(args, b"").unwrap_err() {
            SignError::Failed { stderr, .. } => stderr,
            other => panic!("{other}"),
        };
        assert_eq!(what(&silent), "no output, exit status: 3");
        assert_eq!(what(&blank), "real");
    }
}
