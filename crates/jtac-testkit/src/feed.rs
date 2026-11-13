//! Writing a spawned child's stdin from a test.

use std::io::{ErrorKind, Write as _};
use std::process::Child;

/// Writes `bytes` to the child's piped stdin and closes it. A child that answers before it
/// reads may be gone first, and its closed pipe is its early exit, not the test's failure:
/// unwrapping that write fails on a loaded runner (issue 145). Any other error panics.
///
/// # Panics
/// When stdin was not piped, or the write fails with anything but a closed pipe.
pub fn feed(child: &mut Child, bytes: &[u8]) {
    let mut stdin = child.stdin.take().expect("stdin is piped");
    match stdin.write_all(bytes) {
        Err(e) if e.kind() != ErrorKind::BrokenPipe => panic!("stdin: {e}"),
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use std::process::{Command, Stdio};

    use super::*;

    #[test]
    fn a_child_that_exited_without_reading_is_not_a_failed_write() {
        let (program, args): (&str, &[&str]) = if cfg!(windows) {
            ("cmd", &["/C", "exit"])
        } else {
            ("true", &[])
        };
        let mut child = Command::new(program)
            .args(args)
            .stdin(Stdio::piped())
            .spawn()
            .unwrap();
        while child.try_wait().unwrap().is_none() {
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        feed(&mut child, &[b'x'; 1 << 20]);
        assert!(child.stdin.is_none(), "stdin is closed after the write");
    }

    #[test]
    fn a_child_that_reads_gets_every_byte() {
        let (program, args): (&str, &[&str]) = if cfg!(windows) {
            ("findstr", &["x"])
        } else {
            ("cat", &[])
        };
        let mut child = Command::new(program)
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        feed(&mut child, b"x1\nx2\n");
        let out = child.wait_with_output().unwrap();
        let text = String::from_utf8_lossy(&out.stdout);
        assert_eq!(text.lines().collect::<Vec<_>>(), ["x1", "x2"]);
    }

    /// A read-only file stands in for stdin, so the write fails with something other than
    /// a closed pipe.
    #[cfg(unix)]
    #[test]
    #[should_panic(expected = "stdin:")]
    fn any_other_write_error_panics() {
        use std::os::fd::OwnedFd;

        let mut child = Command::new("cat").stdin(Stdio::piped()).spawn().unwrap();
        let read_only = std::fs::File::open(std::env::current_exe().unwrap()).unwrap();
        child.stdin = Some(OwnedFd::from(read_only).into());
        feed(&mut child, b"x");
    }
}
