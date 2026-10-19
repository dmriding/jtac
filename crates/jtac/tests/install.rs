//! `install.sh` against a release directory built here (DESIGN.md decision 39): the archive
//! named by SHA256SUMS is verified, installed where `--to` says, and a mismatch or a missing
//! archive installs nothing. Unix only: the script is `sh`.
#![cfg(unix)]

use std::path::Path;
use std::process::Command;

use jtac_testkit::Repo;

fn target() -> Option<&'static str> {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("linux", "x86_64") => Some("x86_64-unknown-linux-musl"),
        ("macos", "aarch64") => Some("aarch64-apple-darwin"),
        _ => None,
    }
}

/// A release directory: one archive holding a `jtac` that prints a version, and SHA256SUMS.
fn release(dir: &Path, target: &str) -> String {
    let name = format!("jtac-v9.9.9-{target}");
    let inner = dir.join(&name);
    std::fs::create_dir_all(&inner).unwrap();
    std::fs::write(inner.join("jtac"), "#!/bin/sh\necho jtac 9.9.9\n").unwrap();
    let archive = format!("{name}.tar.gz");
    let tar = Command::new("tar")
        .args(["czf", &archive, &name])
        .current_dir(dir)
        .output()
        .unwrap();
    assert!(
        tar.status.success(),
        "{}",
        String::from_utf8_lossy(&tar.stderr)
    );
    let sums = Command::new("sha256sum")
        .arg(&archive)
        .current_dir(dir)
        .output()
        .unwrap();
    assert!(sums.status.success());
    std::fs::write(dir.join("SHA256SUMS"), &sums.stdout).unwrap();
    archive
}

fn install(base: &Path, to: &Path) -> (i32, String, String) {
    let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../install.sh");
    let out = Command::new("sh")
        .arg(script)
        .arg("--to")
        .arg(to)
        .env("JTAC_INSTALL_BASE", base)
        .env("PATH", std::env::var("PATH").unwrap_or_default())
        .env("HOME", base)
        .output()
        .unwrap();
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

#[test]
fn the_installer_verifies_the_archive_and_installs_nothing_on_a_mismatch() {
    let lab = Repo::empty();
    let to = lab.path().join("bin");
    let Some(target) = target() else {
        let (code, _, stderr) = install(lab.path(), &to);
        assert_eq!(code, 2, "{stderr}");
        assert!(stderr.contains("no prebuilt jtac"), "{stderr}");
        return;
    };
    let archive = release(lab.path(), target);
    let (code, stdout, stderr) = install(lab.path(), &to);
    assert_eq!(code, 0, "{stdout}{stderr}");
    assert!(
        stdout.contains("installed jtac 9.9.9 at")
            && stdout.contains("verified against SHA256SUMS"),
        "{stdout}"
    );
    assert!(stdout.contains("is not on PATH"), "{stdout}");
    assert!(stdout.contains("jtac init"), "{stdout}");
    std::fs::remove_file(to.join("jtac")).unwrap();

    let sums = lab.path().join("SHA256SUMS");
    let good = std::fs::read_to_string(&sums).unwrap();
    let flipped = if good.starts_with('0') { "f" } else { "0" };
    std::fs::write(&sums, format!("{flipped}{}", &good[1..])).unwrap();
    let (code, _, stderr) = install(lab.path(), &to);
    assert_eq!(code, 1, "{stderr}");
    assert!(
        stderr.contains("does not match SHA256SUMS") && stderr.contains("nothing installed"),
        "{stderr}"
    );
    assert!(
        !to.join("jtac").exists(),
        "a mismatch installed the archive"
    );

    std::fs::write(&sums, format!("{} other.tar.gz\n", &good[..64])).unwrap();
    let (code, _, stderr) = install(lab.path(), &to);
    assert_eq!(code, 2, "{stderr}");
    assert!(stderr.contains("lists no archive for"), "{stderr}");
    assert!(!to.join("jtac").exists());
    let _ = archive;
}
