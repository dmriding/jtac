//! Runs the built binary, so the coverage floor measures `main` too.

use std::process::Command;

fn jtac(args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_jtac"))
        .args(args)
        .output()
        .expect("spawn jtac")
}

#[test]
fn binary_prints_its_version_and_exits_zero() {
    let out = jtac(&[]);
    assert!(out.status.success());
    let stdout = String::from_utf8(out.stdout).expect("utf-8");
    assert_eq!(stdout.trim(), format!("jtac {}", env!("CARGO_PKG_VERSION")));
}

#[test]
fn binary_refuses_unbuilt_commands_on_stderr_with_exit_two() {
    let out = jtac(&["check"]);
    assert_eq!(out.status.code(), Some(2));
    assert!(out.stdout.is_empty(), "stdout stays a protocol wire");
    assert!(
        String::from_utf8(out.stderr)
            .expect("utf-8")
            .contains("jtac --version")
    );
}
