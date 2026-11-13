//! The runner against real children. The fixture shells; the product never does.

use std::time::{Duration, Instant};

use jtac_checks::{CAPTURE_CAP, Context, Spec, run};
use jtac_core::{Outcome, Sha256Digest};

fn shell(unix: &str, windows: &str) -> Vec<String> {
    let (bin, flag, script) = if cfg!(windows) {
        ("cmd", "/C", windows)
    } else {
        ("sh", "-c", unix)
    };
    [bin, flag, script].map(String::from).to_vec()
}

fn spec(argv: Vec<String>, timeout: Duration) -> Spec {
    Spec {
        id: "example".into(),
        argv,
        timeout,
    }
}

fn ctx(cap: usize) -> Context {
    Context {
        cwd: std::env::temp_dir(),
        capture_cap: cap,
    }
}

const MINUTE: Duration = Duration::from_secs(60);

/// The deadline for a child whose pid the test reads back: long enough for the child to
/// start and print before the kill, on a loaded runner too (#144). 300 ms was not.
#[cfg(unix)]
const PID_FIRST: Duration = Duration::from_secs(2);

#[cfg(unix)]
fn assert_dead(pid_line: &[u8]) {
    let pid = String::from_utf8_lossy(pid_line).trim().to_string();
    assert!(pid.parse::<u32>().is_ok(), "{pid:?}");
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        let alive = std::process::Command::new("kill")
            .args(["-0", &pid])
            .status()
            .unwrap()
            .success();
        if !alive {
            return;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    panic!("pid {pid} still alive");
}

#[test]
fn exit_zero_passes_and_both_streams_are_captured_and_digested() {
    let ran = run(
        &spec(
            shell("echo out; echo err >&2", "echo out& echo err 1>&2"),
            MINUTE,
        ),
        &ctx(CAPTURE_CAP),
    );
    assert_eq!(ran.body.outcome, Outcome::Pass);
    assert_eq!(ran.body.exit, 0);
    assert_eq!(ran.body.id, "example");
    assert_eq!(ran.body.tool_version, "");
    assert_eq!(ran.body.cached_from, None);
    assert_eq!(String::from_utf8_lossy(&ran.stdout).trim(), "out");
    assert_eq!(String::from_utf8_lossy(&ran.stderr).trim(), "err");
    assert_eq!(ran.body.stdout.sha, Sha256Digest::of(&ran.stdout));
    assert_eq!(ran.body.stderr.sha, Sha256Digest::of(&ran.stderr));
    assert_eq!(ran.body.stdout.bytes, ran.stdout.len() as u64);
    assert!(!ran.body.stdout.truncated && !ran.body.stderr.truncated);
}

#[test]
fn nonzero_exit_fails_with_the_code() {
    let ran = run(&spec(shell("exit 3", "exit 3"), MINUTE), &ctx(CAPTURE_CAP));
    assert_eq!((ran.body.outcome, ran.body.exit), (Outcome::Fail, 3));
}

#[test]
fn a_tool_that_cannot_spawn_is_error_with_empty_captures() {
    let ran = run(
        &spec(vec!["jtac-no-such-tool-7f3a".into()], MINUTE),
        &ctx(CAPTURE_CAP),
    );
    assert_eq!((ran.body.outcome, ran.body.exit), (Outcome::Error, -1));
    assert_eq!(ran.body.stdout.bytes, 0);
    assert_eq!(ran.body.stdout.sha, Sha256Digest::of(b""));
    assert!(!ran.body.stdout.truncated);
    assert_eq!(ran.body.stderr, ran.body.stdout);
    assert_eq!(ran.body.duration_ms, 0);
}

#[test]
fn empty_argv_is_error() {
    let ran = run(&spec(vec![], MINUTE), &ctx(CAPTURE_CAP));
    assert_eq!((ran.body.outcome, ran.body.exit), (Outcome::Error, -1));
}

#[test]
fn output_past_the_cap_is_cut_marked_and_the_child_still_finishes() {
    let ran = run(
        &spec(
            shell(
                "head -c 3000000 /dev/zero | tr '\\0' 'x'",
                "for /L %i in (1,1,100000) do @echo 0123456789012345678901234567890123456789",
            ),
            MINUTE,
        ),
        &ctx(1000),
    );
    assert_eq!(ran.body.outcome, Outcome::Pass);
    assert_eq!(ran.stdout.len(), 1000);
    assert_eq!(ran.body.stdout.bytes, 1000);
    assert!(ran.body.stdout.truncated);
    assert_eq!(ran.body.stdout.sha, Sha256Digest::of(&ran.stdout));
    assert!(!ran.body.stderr.truncated);
}

#[test]
fn a_timeout_kills_the_check_and_is_error() {
    let started = Instant::now();
    let ran = run(
        &spec(
            shell("sleep 30", "waitfor /t 30 jtacTimeout"),
            Duration::from_millis(300),
        ),
        &ctx(CAPTURE_CAP),
    );
    assert_eq!(ran.body.outcome, Outcome::Error);
    assert!(
        started.elapsed() < Duration::from_secs(20),
        "{:?}",
        started.elapsed()
    );
    assert!(ran.body.duration_ms >= 300);
    #[cfg(unix)]
    assert_eq!(ran.body.exit, -9);
}

#[cfg(unix)]
#[test]
fn a_timeout_kills_the_grandchild_too() {
    let started = Instant::now();
    let ran = run(
        &spec(shell("sleep 30 & echo $!; sleep 30", ""), PID_FIRST),
        &ctx(CAPTURE_CAP),
    );
    assert_eq!(ran.body.outcome, Outcome::Error);
    assert!(
        started.elapsed() < Duration::from_secs(20),
        "{:?}",
        started.elapsed()
    );
    assert_dead(&ran.stdout);
}

#[cfg(unix)]
#[test]
fn a_leader_that_leaves_its_group_and_exits_is_reaped_and_passes() {
    let started = Instant::now();
    let argv = [
        "perl",
        "-e",
        "setpgrp(0, getpgrp(getppid())) or die; print qq(done\n); exit 0",
    ]
    .map(String::from)
    .to_vec();
    let ran = run(&spec(argv, Duration::from_secs(10)), &ctx(CAPTURE_CAP));
    assert_eq!((ran.body.outcome, ran.body.exit), (Outcome::Pass, 0));
    assert_eq!(String::from_utf8_lossy(&ran.stdout).trim(), "done");
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "{:?}",
        started.elapsed()
    );
}

#[cfg(unix)]
#[test]
fn a_leader_that_leaves_its_group_is_still_killed_at_the_deadline() {
    let started = Instant::now();
    let argv = [
        "perl",
        "-e",
        "$| = 1; setpgrp(0, getpgrp(getppid())) or die; print qq($$\n); sleep 30",
    ]
    .map(String::from)
    .to_vec();
    let ran = run(&spec(argv, PID_FIRST), &ctx(CAPTURE_CAP));
    assert_eq!((ran.body.outcome, ran.body.exit), (Outcome::Error, -9));
    assert!(
        started.elapsed() < Duration::from_secs(20),
        "{:?}",
        started.elapsed()
    );
    assert_dead(&ran.stdout);
}

/// Two seconds, not 300 ms: the leader's `exit 0` has to land before the deadline, and with
/// the binary's other tests spawning beside it a `cmd` start alone can take longer.
#[test]
fn a_descendant_holding_the_pipe_after_exit_is_error_at_the_deadline() {
    let started = Instant::now();
    let ran = run(
        &spec(
            shell(
                "sleep 30 & echo $!; exit 0",
                "start /b waitfor /t 30 jtacHolder & echo early& exit 0",
            ),
            Duration::from_secs(2),
        ),
        &ctx(CAPTURE_CAP),
    );
    assert_eq!((ran.body.outcome, ran.body.exit), (Outcome::Error, 0));
    assert!(
        started.elapsed() < Duration::from_secs(20),
        "{:?}",
        started.elapsed()
    );
    assert!(ran.body.duration_ms >= 2000);
    #[cfg(unix)]
    assert_dead(&ran.stdout);
    #[cfg(windows)]
    assert_eq!(String::from_utf8_lossy(&ran.stdout).trim(), "early");
}

#[test]
fn stdin_is_closed() {
    let ran = run(
        &spec(shell("cat", "more"), Duration::from_secs(10)),
        &ctx(CAPTURE_CAP),
    );
    assert_eq!((ran.body.outcome, ran.body.exit), (Outcome::Pass, 0));
}

#[test]
fn output_exactly_at_the_cap_is_not_truncated() {
    let argv = shell("echo 0123456789", "echo 0123456789");
    let full = run(&spec(argv.clone(), MINUTE), &ctx(CAPTURE_CAP));
    let len = full.stdout.len();
    let exact = run(&spec(argv.clone(), MINUTE), &ctx(len));
    let short = run(&spec(argv, MINUTE), &ctx(len - 1));
    assert_eq!(
        (exact.stdout.len(), exact.body.stdout.truncated),
        (len, false)
    );
    assert_eq!(
        (short.stdout.len(), short.body.stdout.truncated),
        (len - 1, true)
    );
}

#[cfg(unix)]
#[test]
fn a_signal_death_is_error_with_the_negated_signal() {
    let ran = run(&spec(shell("kill -15 $$", ""), MINUTE), &ctx(CAPTURE_CAP));
    assert_eq!((ran.body.outcome, ran.body.exit), (Outcome::Error, -15));
}

/// The nesting marker is the one name the runner adds itself.
#[test]
fn the_child_sees_only_allowlisted_variables_and_the_nesting_marker() {
    let ran = run(&spec(shell("env", "set"), MINUTE), &ctx(CAPTURE_CAP));
    assert_eq!(ran.body.outcome, Outcome::Pass);
    let out = String::from_utf8_lossy(&ran.stdout);
    let names: Vec<&str> = out
        .lines()
        .filter_map(|l| l.split_once('='))
        .map(|(k, _)| k)
        .collect();
    assert!(
        names.iter().any(|n| n.eq_ignore_ascii_case("PATH")),
        "{out}"
    );
    assert!(!names.iter().any(|n| n.starts_with("CARGO_PKG")), "{out}");
    assert!(
        out.lines()
            .any(|l| l == format!("{}=1", jtac_checks::NESTED_VAR)),
        "{out}"
    );
    for name in names {
        if name == jtac_checks::NESTED_VAR {
            continue;
        }
        let shell_made = ["PWD", "OLDPWD", "SHLVL", "_", "PROMPT", "COMSPEC"]
            .iter()
            .any(|s| s.eq_ignore_ascii_case(name));
        assert!(jtac_checks::allowed(name) || shell_made, "{name} leaked");
    }
}

#[cfg(unix)]
#[test]
fn tool_version_comes_from_the_probe_which_spends_from_the_check_timeout() {
    use std::os::unix::fs::PermissionsExt;
    let dir = std::env::temp_dir().join(format!("jtac-checks-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let rows = [
        (
            "cargo",
            "echo 'cargo 9.9.9 (fixture)'",
            "cargo 9.9.9 (fixture)",
            Outcome::Pass,
        ),
        ("go", "echo 'go version go9.9'; exit 1", "", Outcome::Pass),
        ("node", "sleep 30", "", Outcome::Error),
    ];
    let started = Instant::now();
    for (name, answer, want, outcome) in rows {
        let tool = dir.join(name);
        std::fs::write(&tool, format!("#!/bin/sh\nif [ \"$1\" = --version ] || [ \"$1\" = version ]; then {answer}; fi\nexit 0\n")).unwrap();
        std::fs::set_permissions(&tool, std::fs::Permissions::from_mode(0o755)).unwrap();
        let ran = run(
            &spec(
                vec![tool.to_string_lossy().into_owned(), "test".into()],
                Duration::from_secs(1),
            ),
            &ctx(CAPTURE_CAP),
        );
        assert_eq!(
            (ran.body.outcome, ran.body.tool_version.as_str()),
            (outcome, want),
            "{name}"
        );
    }
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "{:?}",
        started.elapsed()
    );
    std::fs::remove_dir_all(&dir).unwrap();
}
