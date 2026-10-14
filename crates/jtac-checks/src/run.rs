//! Spawn one check: argv only, scrubbed env, capped capture, group kill on timeout
//! (DESIGN.md section 11.1).

use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::process::{ExitStatus, Stdio};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use jtac_core::{Capture, CheckBody, Outcome, Sha256Digest};
use process_wrap::std::{ChildWrapper, CommandWrap};

use crate::env::scrub;
use crate::spec::Spec;
use crate::version::{PROBE_TIMEOUT, first_line, probe_argv};

/// Bytes kept per stream before a capture is cut.
pub const CAPTURE_CAP: usize = 1 << 20;

/// Set to `1` in every check's environment, so `jtac check` inside a check refuses instead of
/// recursing until the timeout (`the_child_sees_only_allowlisted_variables_and_the_nesting_marker`).
pub const NESTED_VAR: &str = "JTAC_CHECK";

const POLL: Duration = Duration::from_millis(10);

const GRACE: Duration = Duration::from_millis(200);

/// Where a check runs and how much of its output is kept.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Context {
    /// The repository root. The runner never discovers it.
    pub cwd: PathBuf,
    /// Per-stream capture cap; `CAPTURE_CAP` outside tests.
    pub capture_cap: usize,
}

/// A finished check: the body, and the bytes its captures digest, for the store to keep.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ran {
    /// The check node body, minus the tree, policy, and `ts` the caller adds.
    pub body: CheckBody,
    /// Captured stdout, already cut at the cap.
    pub stdout: Vec<u8>,
    /// Captured stderr, already cut at the cap.
    pub stderr: Vec<u8>,
}

struct Stream {
    bytes: Vec<u8>,
    truncated: bool,
}

struct Exec {
    exit: i32,
    finished: bool,
    stdout: Stream,
    stderr: Stream,
    duration: Duration,
}

/// Run `spec` in `ctx`. Exit 0 is `pass` and a positive code is `fail`. A timeout, a spawn
/// failure, and a negative code (a signal on Unix, a crash status on Windows) are `error`,
/// never `skipped`. The version probe spends from the same timeout; a probe that exhausts it
/// means the check never starts.
pub fn run(spec: &Spec, ctx: &Context) -> Ran {
    let started = Instant::now();
    let tool_version = probe_argv(&spec.argv)
        .and_then(|argv| execute(&argv, &ctx.cwd, 4096, PROBE_TIMEOUT.min(spec.timeout)).ok())
        .filter(|e| e.finished && e.exit == 0)
        .map_or_else(String::new, |e| first_line(&e.stdout.bytes));
    let left = spec.timeout.saturating_sub(started.elapsed());
    let exec = if left.is_zero() {
        Err(io::ErrorKind::TimedOut.into())
    } else {
        execute(&spec.argv, &ctx.cwd, ctx.capture_cap, left)
    };
    let exec = exec.unwrap_or(Exec {
        exit: -1,
        finished: false,
        stdout: Stream::empty(),
        stderr: Stream::empty(),
        duration: Duration::ZERO,
    });
    let outcome = match exec.exit {
        _ if !exec.finished => Outcome::Error,
        0 => Outcome::Pass,
        1.. => Outcome::Fail,
        _ => Outcome::Error,
    };
    Ran {
        body: CheckBody {
            id: spec.id.clone(),
            argv: spec.argv.clone(),
            tool_version,
            exit: exec.exit,
            outcome,
            stdout: exec.stdout.capture(),
            stderr: exec.stderr.capture(),
            duration_ms: u64::try_from(exec.duration.as_millis()).unwrap_or(u64::MAX),
            cached_from: None,
        },
        stdout: exec.stdout.bytes,
        stderr: exec.stderr.bytes,
    }
}

// `std::env::vars` panics on a non-UTF-8 entry; such an entry is dropped like any other.
fn utf8_vars() -> impl Iterator<Item = (String, String)> {
    std::env::vars_os().filter_map(|(k, v)| Some((k.into_string().ok()?, v.into_string().ok()?)))
}

fn execute(argv: &[String], cwd: &Path, cap: usize, timeout: Duration) -> io::Result<Exec> {
    let (argv0, args) = argv
        .split_first()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "empty argv"))?;
    let mut command = CommandWrap::with_new(argv0, |c| {
        c.args(args)
            .current_dir(cwd)
            .env_clear()
            .envs(scrub(utf8_vars()))
            .env(NESTED_VAR, "1")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
    });
    #[cfg(unix)]
    command.wrap(process_wrap::std::ProcessGroup::leader());
    #[cfg(windows)]
    command.wrap(process_wrap::std::JobObject);
    let started = Instant::now();
    let deadline = started + timeout;
    let mut child = command.spawn()?;
    let stdout = drain(child.stdout().take(), cap);
    let stderr = drain(child.stderr().take(), cap);
    let (status, mut finished) = wait_until(child.as_mut(), deadline);
    if !settled(&[&stdout, &stderr], deadline) {
        kill_all(child.as_mut());
        finished = false;
        settled(&[&stdout, &stderr], Instant::now() + GRACE);
    }
    Ok(Exec {
        exit: status.map_or(-1, exit_code),
        finished,
        stdout: stdout.take(),
        stderr: stderr.take(),
        duration: started.elapsed(),
    })
}

// The leader's exit reaps nothing it left behind; `settled` is what bounds those.
fn wait_until(child: &mut dyn ChildWrapper, deadline: Instant) -> (Option<ExitStatus>, bool) {
    loop {
        match poll(child) {
            Ok(Some(status)) => return (Some(status), true),
            Ok(None) if Instant::now() < deadline => thread::sleep(POLL),
            _ => break,
        }
    }
    kill_all(child);
    let reaped = Instant::now() + GRACE;
    loop {
        match poll(child) {
            Ok(Some(status)) => return (Some(status), false),
            Ok(None) if Instant::now() < reaped => thread::sleep(POLL),
            _ => return (None, false),
        }
    }
}

// A leader that left its group is invisible to the group wait; the pid wait still sees it.
fn poll(child: &mut dyn ChildWrapper) -> io::Result<Option<ExitStatus>> {
    match child.try_wait()? {
        Some(status) => Ok(Some(status)),
        None => child.inner_mut().try_wait(),
    }
}

// A child that left its group makes the group kill miss, so the leader is killed by pid too.
fn kill_all(child: &mut dyn ChildWrapper) {
    let _ = child.start_kill();
    let _ = child.inner_mut().start_kill();
}

// True once every reader hit EOF; false at `deadline` while a descendant still holds a pipe.
fn settled(readers: &[&Reader], deadline: Instant) -> bool {
    loop {
        if readers.iter().all(|r| r.handle.is_finished()) {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        thread::sleep(POLL);
    }
}

struct Reader {
    stream: Arc<Mutex<Stream>>,
    handle: JoinHandle<()>,
}

impl Reader {
    fn take(&self) -> Stream {
        std::mem::replace(
            &mut self.stream.lock().unwrap_or_else(|e| e.into_inner()),
            Stream::empty(),
        )
    }
}

// The reader keeps draining past the cap so a chatty check never blocks on a full pipe, and
// writes through a lock so a run can take what arrived when the pipe never closes.
fn drain<R: Read + Send + 'static>(reader: Option<R>, cap: usize) -> Reader {
    let stream = Arc::new(Mutex::new(Stream::empty()));
    let shared = Arc::clone(&stream);
    let handle = thread::spawn(move || {
        let Some(mut reader) = reader else {
            return;
        };
        let mut chunk = [0u8; 8192];
        loop {
            let n = match reader.read(&mut chunk) {
                Ok(0) => return,
                Ok(n) => n,
                Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                Err(_) => return,
            };
            let mut s = shared.lock().unwrap_or_else(|e| e.into_inner());
            let room = cap.saturating_sub(s.bytes.len());
            s.bytes.extend_from_slice(&chunk[..n.min(room)]);
            s.truncated |= n > room;
        }
    });
    Reader { stream, handle }
}

#[cfg(unix)]
fn exit_code(status: ExitStatus) -> i32 {
    use std::os::unix::process::ExitStatusExt;
    status
        .code()
        .or_else(|| status.signal().map(|s| -s))
        .unwrap_or(-1)
}

#[cfg(not(unix))]
fn exit_code(status: ExitStatus) -> i32 {
    status.code().unwrap_or(-1)
}

impl Stream {
    fn empty() -> Self {
        Self {
            bytes: Vec::new(),
            truncated: false,
        }
    }

    fn capture(&self) -> Capture {
        Capture {
            sha: Sha256Digest::of(&self.bytes),
            bytes: self.bytes.len() as u64,
            truncated: self.truncated,
        }
    }
}
