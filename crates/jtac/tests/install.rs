//! `install.sh` against a release directory built here (DESIGN.md decision 39): it installs only
//! the archive a release attestation signed in `jtac-release-v1` by the key it carries names,
//! where `--to` says. Tests run a copy of the script carrying a throwaway key, so the product has
//! no trust override. Unix only: the script is `sh`.
#![cfg(unix)]

use std::ffi::OsStr;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;

use jtac_testkit::Repo;

const SCRIPT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../install.sh");
const SIGNERS_OPEN: &str = "<<'SIGNERS'\n";
const SIGNERS_CLOSE: &str = "\nSIGNERS\n";
const CI: &str = "jtac-ci@users.noreply.github.com";

fn target() -> Option<&'static str> {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("linux", "x86_64") => Some("x86_64-unknown-linux-musl"),
        ("macos", "aarch64") => Some("aarch64-apple-darwin"),
        _ => None,
    }
}

fn run(cmd: &mut Command) -> std::process::Output {
    let out = cmd.output().unwrap();
    assert!(
        out.status.success(),
        "{cmd:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    out
}

/// The signers block between its heredoc markers, as the script carries it.
fn signers_block(script: &str) -> &str {
    let start = script.find(SIGNERS_OPEN).expect("the signers heredoc") + SIGNERS_OPEN.len();
    let len = script[start..].find(SIGNERS_CLOSE).expect("its close");
    &script[start..start + len]
}

/// A throwaway ed25519 key and the signers line `options` puts it under.
fn key(dir: &Path, name: &str, options: &str) -> (PathBuf, String) {
    let path = dir.join(name);
    run(Command::new("ssh-keygen")
        .args(["-q", "-t", "ed25519", "-N", "", "-C", name, "-f"])
        .arg(&path));
    let public = std::fs::read_to_string(path.with_extension("pub")).unwrap();
    let public: Vec<&str> = public.split_whitespace().take(2).collect();
    (path, format!("{CI} {options}{}", public.join(" ")))
}

/// install.sh with `line` in place of the release signers it ships.
fn script_trusting(dir: &Path, line: &str) -> PathBuf {
    let shipped = std::fs::read_to_string(SCRIPT).unwrap();
    let block = signers_block(&shipped);
    let path = dir.join("install.sh");
    std::fs::write(&path, shipped.replacen(block, line, 1)).unwrap();
    path
}

fn sha256(dir: &Path, file: &str) -> String {
    [("sha256sum", &[][..]), ("shasum", &["-a", "256"][..])]
        .into_iter()
        .find_map(|(tool, flags)| {
            Command::new(tool)
                .args(flags)
                .arg(file)
                .current_dir(dir)
                .output()
                .ok()
                .filter(|o| o.status.success())
        })
        .map(|o| String::from_utf8(o.stdout).unwrap()[..64].to_owned())
        .expect("sha256sum or shasum, the tools the installer verifies with")
}

/// A release page: one archive holding a `jtac` that prints `body`, and the `SHA256SUMS` the
/// workflow publishes beside it. Returns the archive's name.
fn archive(dir: &Path, target: &str, version: &str, body: &str) -> String {
    let name = format!("jtac-{version}-{target}");
    let inner = dir.join(&name);
    std::fs::create_dir_all(&inner).unwrap();
    std::fs::write(inner.join("jtac"), format!("#!/bin/sh\necho {body}\n")).unwrap();
    let archive = format!("{name}.tar.gz");
    run(Command::new("tar")
        .args(["czf", &archive, &name])
        .current_dir(dir));
    let sums = format!("{}  {archive}\n", sha256(dir, &archive));
    std::fs::write(dir.join("SHA256SUMS"), sums).unwrap();
    archive
}

/// `jtac-release.json` naming `artifacts` as `(name, digest)`, in the release document's shape,
/// signed by `key` in `namespace` as `jtac-release.sig`. The `commit` after the last artifact is
/// a sha256 id, so a parse that reads the last `sha256:` takes the wrong digest.
fn attest(dir: &Path, key: &Path, namespace: &str, artifacts: &[(&str, &str)]) {
    let listed: Vec<String> = artifacts
        .iter()
        .map(|(name, digest)| format!(r#"{{"name":"{name}","sha256":"sha256:{digest}"}}"#))
        .collect();
    let doc = format!(
        r#"{{"artifacts":[{}],"ci":true,"commit":"sha256:{}","from":{{"tag":"v9.9.8"}},"tag":"v9.9.9","v":1}}"#,
        listed.join(","),
        "7".repeat(64)
    );
    let json = dir.join("jtac-release.json");
    std::fs::write(&json, doc).unwrap();
    let sig = dir.join("jtac-release.json.sig");
    let _ = std::fs::remove_file(&sig);
    run(Command::new("ssh-keygen")
        .args(["-Y", "sign", "-n", namespace, "-f"])
        .arg(key)
        .arg(&json));
    std::fs::rename(sig, dir.join("jtac-release.sig")).unwrap();
}

fn installer(script: &Path, base: impl AsRef<OsStr>, to: &Path) -> Command {
    let mut cmd = Command::new("sh");
    cmd.arg(script)
        .arg("--to")
        .arg(to)
        .env("JTAC_INSTALL_BASE", base)
        .env("PATH", std::env::var_os("PATH").unwrap_or_default())
        .env("HOME", script.parent().unwrap());
    cmd
}

fn install(script: &Path, base: &Path, to: &Path, args: &[&str]) -> (i32, String, String) {
    outcome(installer(script, base, to).args(args))
}

fn outcome(cmd: &mut Command) -> (i32, String, String) {
    let out = cmd.output().unwrap();
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

fn refused(result: (i32, String, String), code: i32, says: &str, to: &Path) {
    let (got, stdout, stderr) = result;
    assert_eq!(got, code, "{says}: {stdout}{stderr}");
    assert!(
        stderr.contains(says) && stderr.contains("nothing installed"),
        "{stderr}"
    );
    let refusal: Vec<&str> = stderr
        .lines()
        .skip_while(|l| !l.starts_with("install.sh:"))
        .collect();
    assert!(refusal.len() >= 3, "what, why and next: {stderr}");
    assert!(!to.join("jtac").exists(), "{says}: installed anyway");
}

#[test]
fn the_shipped_release_signers_are_the_trust_files_release_lines() {
    let shipped = std::fs::read_to_string(SCRIPT).unwrap();
    let mut carried: Vec<&str> = signers_block(&shipped).lines().collect();
    let trust =
        std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../.jtac/signers"))
            .unwrap();
    let mut release: Vec<&str> = trust
        .lines()
        .filter(|line| {
            line.split_whitespace()
                .nth(1)
                .and_then(|o| o.strip_prefix("namespaces=\""))
                .and_then(|o| o.strip_suffix('"'))
                .is_some_and(|list| list.split(',').any(|n| n == "jtac-release-v1"))
        })
        .collect();
    carried.sort_unstable();
    release.sort_unstable();
    assert!(!release.is_empty(), "no signers line names jtac-release-v1");
    assert_eq!(
        carried, release,
        "install.sh must carry exactly the jtac-release-v1 lines of .jtac/signers"
    );
}

#[test]
fn the_installer_verifies_the_archive_and_installs_nothing_on_a_mismatch() {
    let lab = Repo::empty();
    let dir = lab.path();
    let to = dir.join("bin");
    let (ci, line) = key(dir, "ci", "namespaces=\"jtac-ci-root-v1,jtac-release-v1\" ");
    let script = script_trusting(dir, &line);
    let Some(target) = target() else {
        let (code, _, stderr) = install(&script, dir, &to, &[]);
        assert_eq!(code, 2, "{stderr}");
        assert!(stderr.contains("no prebuilt jtac"), "{stderr}");
        return;
    };
    let other = "jtac-v9.9.9-x86_64-pc-windows-msvc.zip";
    let good = archive(dir, target, "v9.9.9", "jtac 9.9.9");
    let digest = sha256(dir, &good);
    attest(
        dir,
        &ci,
        "jtac-release-v1",
        &[(other, &"0".repeat(64)), (&good, &digest)],
    );
    let (code, stdout, stderr) = install(&script, dir, &to, &["--version", "v9.9.9"]);
    assert_eq!(code, 0, "{stdout}{stderr}");
    assert!(
        stdout.contains("installed jtac 9.9.9 at")
            && stdout.contains(&format!(
                "verified against the release attestation {CI} signed"
            )),
        "{stdout}"
    );
    assert!(
        stdout.contains("is not on PATH") && stdout.contains("jtac init"),
        "{stdout}"
    );
    std::fs::remove_file(to.join("jtac")).unwrap();

    let held = std::fs::read(dir.join(&good)).unwrap();
    assert_eq!(archive(dir, target, "v9.9.9", "swapped"), good);
    let r = install(&script, dir, &to, &[]);
    refused(r, 1, "does not match the signed release attestation", &to);
    std::fs::write(dir.join(&good), held).unwrap();

    let r = install(&script, dir, &to, &["--version", "v9.9.8"]);
    refused(r, 1, &format!("is for {good}, not v9.9.8"), &to);

    let (developer, _) = key(dir, "developer", "");
    attest(dir, &developer, "jtac-release-v1", &[(&good, &digest)]);
    let r = install(&script, dir, &to, &[]);
    refused(r, 1, "not signed by jtac's release key", &to);
    attest(dir, &ci, "jtac-ci-root-v1", &[(&good, &digest)]);
    let r = install(&script, dir, &to, &[]);
    refused(r, 1, "not signed by jtac's release key", &to);

    attest(dir, &ci, "jtac-release-v1", &[(other, &digest)]);
    let r = install(&script, dir, &to, &[]);
    refused(r, 2, "does not name exactly one archive", &to);
    attest(
        dir,
        &ci,
        "jtac-release-v1",
        &[(&good, &digest), (&good, &digest)],
    );
    let r = install(&script, dir, &to, &[]);
    refused(r, 2, "does not name exactly one archive", &to);

    for missing in ["jtac-release.json", "jtac-release.sig"] {
        attest(dir, &ci, "jtac-release-v1", &[(&good, &digest)]);
        std::fs::remove_file(dir.join(missing)).unwrap();
        let r = install(&script, dir, &to, &[]);
        refused(r, 2, "v0.2.7 and older", &to);
    }
}

#[test]
fn the_installer_installs_nothing_it_cannot_fetch_or_write() {
    let lab = Repo::empty();
    let dir = lab.path();
    let to = dir.join("bin");
    let (ci, line) = key(dir, "ci", "namespaces=\"jtac-release-v1\" ");
    let script = script_trusting(dir, &line);
    let Some(target) = target() else { return };
    let good = archive(dir, target, "v9.9.9", "jtac 9.9.9");
    attest(dir, &ci, "jtac-release-v1", &[(&good, &sha256(dir, &good))]);
    let (code, stdout, stderr) = install(&script, dir, &to, &[]);
    assert_eq!(code, 0, "{stdout}{stderr}");
    let (code, stdout, stderr) = install(&script, dir, &to, &[]);
    assert_eq!(
        code, 0,
        "an upgrade over the installed file: {stdout}{stderr}"
    );
    assert!(
        !to.join(".jtac.new").exists(),
        "the staging file is moved into place"
    );
    std::fs::remove_file(to.join("jtac")).unwrap();

    std::fs::rename(dir.join(&good), dir.join("held")).unwrap();
    refused(install(&script, dir, &to, &[]), 2, "could not fetch", &to);
    std::fs::rename(dir.join("held"), dir.join(&good)).unwrap();

    std::fs::create_dir_all(to.join("jtac")).unwrap();
    let (code, _, stderr) = install(&script, dir, &to, &[]);
    assert_eq!(code, 2, "a directory where the binary goes: {stderr}");
    assert!(stderr.contains("could not write"), "{stderr}");
    assert!(to.join("jtac").is_dir() && !to.join("jtac").join(".jtac.new").exists());
    std::fs::remove_dir(to.join("jtac")).unwrap();
    let sealed = dir.join("sealed");
    std::fs::create_dir(&sealed).unwrap();
    std::fs::set_permissions(&sealed, std::fs::Permissions::from_mode(0o555)).unwrap();
    let (code, _, stderr) = install(&script, dir, &sealed.join("bin"), &[]);
    std::fs::set_permissions(&sealed, std::fs::Permissions::from_mode(0o755)).unwrap();
    if !sealed.join("bin").exists() {
        assert_eq!(code, 2, "a destination that refuses the write: {stderr}");
        assert!(stderr.contains("could not write"), "{stderr}");
    }
}

/// A directory of links to `tools` alone, for a PATH that holds nothing else.
fn bare_path(dir: &Path, tools: &[&str]) -> PathBuf {
    let bare = dir.join("bare");
    std::fs::create_dir_all(&bare).unwrap();
    let path = std::env::var_os("PATH").unwrap_or_default();
    for tool in tools {
        let found = std::env::split_paths(&path)
            .map(|p| p.join(tool))
            .find(|p| p.is_file())
            .unwrap_or_else(|| panic!("{tool} on PATH"));
        std::os::unix::fs::symlink(found, bare.join(tool)).unwrap();
    }
    bare
}

#[test]
fn the_installer_refuses_without_an_ssh_keygen_that_verifies() {
    let lab = Repo::empty();
    let dir = lab.path();
    let to = dir.join("bin");
    let (ci, line) = key(dir, "ci", "namespaces=\"jtac-release-v1\" ");
    let script = script_trusting(dir, &line);
    let Some(target) = target() else { return };
    let good = archive(dir, target, "v9.9.9", "jtac 9.9.9");
    attest(dir, &ci, "jtac-release-v1", &[(&good, &sha256(dir, &good))]);

    let bare = bare_path(dir, &["sh", "uname"]);
    let r = outcome(installer(&script, dir, &to).env("PATH", bare));
    refused(r, 2, "needs ssh-keygen", &to);

    let old = dir.join("old");
    std::fs::create_dir(&old).unwrap();
    let stub = old.join("ssh-keygen");
    std::fs::write(&stub, "#!/bin/sh\necho 'unknown option -- Y' >&2\nexit 1\n").unwrap();
    std::fs::set_permissions(&stub, std::fs::Permissions::from_mode(0o755)).unwrap();
    let path = std::env::var_os("PATH").unwrap_or_default();
    let path = std::env::join_paths(std::iter::once(old).chain(std::env::split_paths(&path)));
    let r = outcome(installer(&script, dir, &to).env("PATH", path.unwrap()));
    refused(
        r,
        2,
        "could not check the release attestation (exit 1)",
        &to,
    );
}

#[test]
fn the_installer_reads_a_url_base_as_a_url_never_as_a_directory() {
    let lab = Repo::empty();
    let dir = lab.path();
    let to = dir.join("bin");
    let (ci, line) = key(dir, "ci", "namespaces=\"jtac-release-v1\" ");
    let script = script_trusting(dir, &line);
    let Some(target) = target() else { return };
    let decoy = dir.join("https:/example.invalid/download");
    std::fs::create_dir_all(&decoy).unwrap();
    let good = archive(&decoy, target, "v9.9.9", "jtac 9.9.9");
    attest(
        &decoy,
        &ci,
        "jtac-release-v1",
        &[(&good, &sha256(&decoy, &good))],
    );
    let bare = bare_path(dir, &["sh", "uname", "ssh-keygen", "mktemp", "cat", "rm"]);
    let mut cmd = installer(&script, "https://example.invalid/download", &to);
    let r = outcome(cmd.env("PATH", bare).current_dir(dir));
    refused(r, 2, "needs curl or wget", &to);
}
