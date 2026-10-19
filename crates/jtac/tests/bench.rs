//! `jtac bench` against the binary (DESIGN.md section 13): every control holds, each row
//! quotes the gate's own line, and `--json` carries the same rows.

use jtac_testkit::Repo;
use serde_json::Value;

#[test]
fn every_control_holds_and_each_row_quotes_the_gate() {
    let cwd = Repo::empty();
    let tmp = cwd.path().join("tmp");
    std::fs::create_dir(&tmp).unwrap();
    let tmp = tmp.to_string_lossy().into_owned();
    let out = cwd.command_with(env!("CARGO_BIN_EXE_jtac"), &["bench"], &[("TMPDIR", &tmp)]);
    let stdout = String::from_utf8(out.stdout).unwrap();
    let stderr = String::from_utf8(out.stderr).unwrap();
    assert_eq!(out.status.code(), Some(0), "{stdout}{stderr}");
    assert!(
        stdout.starts_with("jtac bench: 10 of 10 controls held\n"),
        "{stdout}"
    );
    for want in [
        "CLEARED HOT",
        "has no root",
        "its signature does not hold",
        "names tree",
        "JT-TEST-001",
        "JT-SUP-001",
        "silent by design; jtac: `git push --no-verify` skips the pre-push hook",
        "check `self` error",
        "rewinds a protected branch",
    ] {
        assert!(stdout.contains(want), "{want}: {stdout}");
    }
    assert_eq!(
        std::fs::read_dir(&tmp).unwrap().count(),
        0,
        "the lab and every child's temp file are gone with the run"
    );
    let out = cwd.command(env!("CARGO_BIN_EXE_jtac"), &["bench", "--json"]);
    assert_eq!(out.status.code(), Some(0));
    let v: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["v"], 1);
    assert_eq!(v["pass"], true);
    let rows = v["rows"].as_array().unwrap();
    assert_eq!(rows.len(), 10);
    assert!(rows.iter().all(|r| r["held"] == true), "{v}");
}
