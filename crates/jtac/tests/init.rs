//! `jtac init` against throwaway repositories (DESIGN.md section 11.7), with the global git
//! config and the user config directory in a second temp directory, through the built binary:
//! what a first run writes, that a second and a dry run write nothing, every key source, the
//! hook rules, remote choice, and detection.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use jtac_policy::{Detected, Js, Runner, default_policy, policy_hash};
use jtac_testkit::Repo;
use serde_json::Value;

const REFSPEC: &str = "+refs/jtac/*:refs/jtac/*";

fn slashes(p: &Path) -> String {
    p.to_string_lossy().replace('\\', "/")
}

/// A path the way git and init print it: canonical, forward slashes, no verbatim prefix
/// (a Windows temp dir is an 8.3 short name, and git reports the long one).
fn canon(p: &Path) -> String {
    let text = slashes(&std::fs::canonicalize(p).unwrap());
    text.strip_prefix("//?/")
        .map_or(text.clone(), str::to_owned)
}

fn exe() -> String {
    canon(Path::new(env!("CARGO_BIN_EXE_jtac")))
}

fn hook_text(exe: &str) -> String {
    format!("#!/bin/sh\n# jtac-hook v1\nexec '{exe}' hook pre-push \"$@\"\n")
}

struct Ran {
    code: i32,
    stdout: String,
    stderr: String,
}

/// A repository and a home: the global git config and `XDG_CONFIG_HOME` live in the home, so
/// a key init generates and the config it sets never touch the real ones.
struct Env {
    repo: Repo,
    home: Repo,
}

impl Env {
    fn bare() -> Self {
        let repo = Repo::init(&[]);
        repo.git(&["config", "user.email", "dev@example.com"]);
        Self {
            repo,
            home: Repo::empty(),
        }
    }

    /// With `origin`, the common case.
    fn new() -> Self {
        let env = Self::bare();
        env.git(&[
            "remote",
            "add",
            "origin",
            "https://example.com/example-app.git",
        ]);
        env
    }

    fn global(&self) -> String {
        slashes(&self.home.path().join("gitconfig"))
    }

    fn xdg(&self) -> PathBuf {
        self.home.path().join("xdg")
    }

    fn key(&self) -> PathBuf {
        self.xdg().join("jtac").join("id_ed25519")
    }

    fn run(&self, program: &str, args: &[&str]) -> std::process::Output {
        let (global, xdg) = (self.global(), slashes(&self.xdg()));
        let extra = [
            ("GIT_CONFIG_GLOBAL", global.as_str()),
            ("XDG_CONFIG_HOME", xdg.as_str()),
            ("APPDATA", xdg.as_str()),
        ];
        self.repo.command_with(program, args, &extra)
    }

    fn jtac(&self, args: &[&str]) -> Ran {
        let out = self.run(env!("CARGO_BIN_EXE_jtac"), args);
        Ran {
            code: out.status.code().unwrap_or(-1),
            stdout: String::from_utf8(out.stdout).unwrap(),
            stderr: String::from_utf8(out.stderr).unwrap(),
        }
    }

    /// git under the same global config; trimmed stdout, empty on a non-zero exit.
    fn git(&self, args: &[&str]) -> String {
        let out = self.run("git", args);
        if !out.status.success() {
            return String::new();
        }
        String::from_utf8_lossy(&out.stdout).trim().to_owned()
    }

    fn init(&self, args: &[&str]) -> Ran {
        let ran = self.jtac(&[&["init"], args].concat());
        assert_eq!(ran.code, 0, "{}{}", ran.stdout, ran.stderr);
        ran
    }

    fn objects(&self) -> Vec<PathBuf> {
        let dir = self.repo.path().join(".git").join("jtac").join("objects");
        let Ok(entries) = std::fs::read_dir(dir) else {
            return Vec::new();
        };
        entries.map(|e| e.unwrap().path()).collect()
    }

    /// Every file and directory under the repository and the home, bytes included.
    fn snapshot(&self) -> BTreeMap<PathBuf, Vec<u8>> {
        let mut out = BTreeMap::new();
        let mut stack = vec![
            self.repo.path().to_path_buf(),
            self.home.path().to_path_buf(),
        ];
        while let Some(path) = stack.pop() {
            let meta = std::fs::symlink_metadata(&path).unwrap();
            if meta.is_dir() {
                stack.extend(std::fs::read_dir(&path).unwrap().map(|e| e.unwrap().path()));
                out.insert(path, Vec::new());
            } else if path.extension().is_none_or(|e| e != "profraw") {
                out.insert(path.clone(), std::fs::read(&path).unwrap());
            }
        }
        out
    }
}

/// A key made the way a developer who signs commits with SSH already has one.
fn ssh_key(at: &Path) -> String {
    let out = Command::new("ssh-keygen")
        .args([
            "-q",
            "-t",
            "ed25519",
            "-N",
            "",
            "-C",
            "dev@example.com",
            "-f",
        ])
        .arg(at)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    slashes(at)
}

#[test]
fn a_first_run_writes_every_step_and_the_generated_key_signs_a_root() {
    let env = Env::new();
    env.repo
        .write("Cargo.toml", b"[package]\nname = \"example-app\"\n");
    let ran = env.init(&[]);
    let key = slashes(&env.key());
    let hook = env.repo.path().join(".git").join("hooks").join("pre-push");
    for line in [
        "jtac init: 6 written, 0 kept\n".to_owned(),
        format!("  wrote signing key {key} (generated, SHA256:"),
        "  wrote store ".to_owned(),
        "  wrote policy .jtac/policy.toml (detected: rust)\n".to_owned(),
        format!("  wrote pre-push hook {}\n", canon(&hook)),
        format!("  wrote fetch refspec {REFSPEC} on origin\n"),
        "  wrote lever node sha256:".to_owned(),
        "  allowed signer: dev@example.com ssh-ed25519 ".to_owned(),
        "    [checks.rust-test]\n".to_owned(),
    ] {
        assert!(ran.stdout.contains(&line), "{line}\n{}", ran.stdout);
    }
    assert!(env.key().is_file() && env.key().with_extension("pub").is_file());
    assert_eq!(env.git(&["config", "--global", "jtac.signingkey"]), key);
    assert_eq!(std::fs::read_to_string(&hook).unwrap(), hook_text(&exe()));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&hook).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o755);
    }
    let fetch = env.git(&["config", "--get-all", "remote.origin.fetch"]);
    assert_eq!(
        fetch.lines().filter(|l| *l == REFSPEC).count(),
        1,
        "{fetch}"
    );
    let text = std::fs::read_to_string(env.repo.path().join(".jtac/policy.toml")).unwrap();
    let rust = Detected {
        rust: true,
        ..Detected::default()
    };
    let parsed = jtac_policy::parse(&text).unwrap();
    assert_eq!(
        policy_hash(&parsed).unwrap(),
        policy_hash(&default_policy(&rust)).unwrap()
    );
    let objects = env.objects();
    assert_eq!(objects.len(), 1, "{objects:?}");
    let node: Value = serde_json::from_slice(&std::fs::read(&objects[0]).unwrap()).unwrap();
    assert_eq!(
        (node["kind"].as_str(), node["body"]["action"].as_str()),
        (Some("lever"), Some("init"))
    );
    assert_eq!(node["body"]["signer"]["principal"], "dev@example.com");
    assert_eq!(node["policy"], policy_hash(&parsed).unwrap().to_string());
    assert_eq!(
        node["tree"],
        "sha1:4b825dc642cb6eb9a060e54bf8d69288fbee4904"
    );
    let name = objects[0]
        .file_name()
        .unwrap()
        .to_string_lossy()
        .into_owned();
    assert!(
        ran.stdout.contains(&format!("lever node sha256:{name}")),
        "{}",
        ran.stdout
    );

    let policy = format!(
        "version = 1\n[checks.ok]\nrun = [\"{}\", \"--version\"]\n",
        slashes(Path::new(env!("CARGO_BIN_EXE_jtac")))
    );
    env.repo.write(".jtac/policy.toml", policy.as_bytes());
    env.repo.write(".gitignore", b"*.profraw\n");
    env.git(&["add", "-A"]);
    env.git(&["commit", "-q", "--no-gpg-sign", "-m", "base"]);
    let head = env.git(&["rev-parse", "HEAD"]);
    let ran = env.jtac(&["check"]);
    assert_eq!(ran.code, 0, "{}{}", ran.stdout, ran.stderr);
    assert!(ran.stdout.starts_with("CLEARED HOT: "), "{}", ran.stdout);
    let refs = env.git(&["for-each-ref", "--format=%(refname)", "refs/jtac/roots/"]);
    assert!(refs.ends_with(&head), "{refs}");
}

#[test]
fn a_second_run_keeps_every_step_and_changes_nothing() {
    let env = Env::new();
    env.repo
        .write("go.mod", b"module example.com/example-app\n");
    env.init(&[]);
    let before = env.snapshot();
    let ran = env.init(&[]);
    assert_eq!(env.snapshot(), before, "{}", ran.stdout);
    assert!(
        ran.stdout.starts_with("jtac init: nothing to do, 6 kept\n"),
        "{}",
        ran.stdout
    );
    assert!(!ran.stdout.contains("wrote"), "{}", ran.stdout);
    assert!(ran.stdout.contains("  kept signing key "), "{}", ran.stdout);
    assert!(
        ran.stdout.contains("named by jtac.signingkey)"),
        "{}",
        ran.stdout
    );
    assert!(
        ran.stdout.contains("  kept lever node: nothing changed"),
        "{}",
        ran.stdout
    );
    assert_eq!(env.objects().len(), 1);
}

#[test]
fn a_dry_run_reports_would_write_and_writes_nothing() {
    let env = Env::new();
    env.repo
        .write("Cargo.toml", b"[package]\nname = \"example-app\"\n");
    let before = env.snapshot();
    let ran = env.init(&["--dry-run"]);
    assert_eq!(env.snapshot(), before, "{}", ran.stdout);
    assert!(
        ran.stdout
            .starts_with("jtac init (dry run, nothing written): 6 would be written, 0 kept\n"),
        "{}",
        ran.stdout
    );
    for step in [
        "signing key",
        "store",
        "policy",
        "pre-push hook",
        "fetch refspec",
        "lever node",
    ] {
        assert!(
            ran.stdout.contains(&format!("  would write {step} ")),
            "{step}\n{}",
            ran.stdout
        );
    }
    assert!(!env.key().exists());
}

/// An unusable file at the default key path is refused before the global config names it,
/// so the retry the message gives does not land on the same file again.
#[test]
fn an_unusable_file_at_the_key_path_refuses_and_names_nothing() {
    let env = Env::new();
    std::fs::create_dir_all(env.key().parent().unwrap()).unwrap();
    std::fs::write(env.key(), b"").unwrap();
    let ran = env.jtac(&["init"]);
    assert_eq!(ran.code, 2, "{}{}", ran.stdout, ran.stderr);
    assert!(ran.stderr.contains("does not work"), "{}", ran.stderr);
    assert!(
        ran.stderr.contains("Move the file aside, `mv "),
        "{}",
        ran.stderr
    );
    assert_eq!(env.git(&["config", "--global", "jtac.signingkey"]), "");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::remove_file(env.key()).unwrap();
        let out = std::process::Command::new("ssh-keygen")
            .args(["-q", "-t", "ed25519", "-N", "", "-f"])
            .arg(env.key())
            .output()
            .unwrap();
        assert!(out.status.success());
        std::fs::set_permissions(env.key(), std::fs::Permissions::from_mode(0o644)).unwrap();
        let ran = env.jtac(&["init"]);
        assert_eq!(
            ran.code, 2,
            "a key ssh-keygen will not sign with: {}",
            ran.stdout
        );
        assert!(ran.stderr.contains("does not work"), "{}", ran.stderr);
        assert_eq!(env.git(&["config", "--global", "jtac.signingkey"]), "");
    }
}

#[test]
fn a_commit_signing_ssh_key_is_kept_and_nothing_is_generated() {
    let env = Env::new();
    let key = ssh_key(&env.repo.path().join(".git").join("key"));
    env.git(&["config", "user.signingkey", &key]);
    env.git(&["config", "gpg.format", "ssh"]);
    let ran = env.init(&[]);
    assert!(
        ran.stdout
            .contains(&format!("  kept signing key {key} (SHA256:")),
        "{}",
        ran.stdout
    );
    assert!(
        ran.stdout
            .contains("named by user.signingkey with gpg.format ssh)"),
        "{}",
        ran.stdout
    );
    assert!(
        ran.stdout.contains("jtac init: 5 written, 1 kept"),
        "{}",
        ran.stdout
    );
    assert!(!env.xdg().exists());
    assert!(!env.home.path().join("gitconfig").exists());
}

#[test]
fn jtac_signingkey_wins_over_the_commit_signing_key() {
    let env = Env::new();
    let git_key = ssh_key(&env.repo.path().join(".git").join("git-key"));
    let jtac_key = ssh_key(&env.repo.path().join(".git").join("jtac-key"));
    env.git(&["config", "user.signingkey", &git_key]);
    env.git(&["config", "gpg.format", "ssh"]);
    env.git(&["config", "jtac.signingkey", &jtac_key]);
    let ran = env.init(&[]);
    assert!(
        ran.stdout
            .contains(&format!("  kept signing key {jtac_key} (SHA256:")),
        "{}",
        ran.stdout
    );
    assert!(
        ran.stdout.contains("named by jtac.signingkey)"),
        "{}",
        ran.stdout
    );
    assert!(!env.xdg().exists());
    std::fs::remove_file(env.repo.path().join(".git").join("jtac-key.pub")).unwrap();
    let ran = env.init(&[]);
    assert!(!ran.stdout.contains("allowed signer:"), "{}", ran.stdout);
    let shown = jtac_checks::git::quoted(&jtac_key, "<key path>");
    let hint = format!("(`ssh-keygen -y -f {shown}` prints the public key");
    assert!(ran.stdout.contains(&hint), "{}", ran.stdout);
}

#[test]
fn no_user_email_refuses_naming_the_command_and_writes_nothing() {
    let env = Env::new();
    env.git(&["config", "--unset", "user.email"]);
    let before = env.snapshot();
    let ran = env.jtac(&["init"]);
    assert_eq!(ran.code, 2, "{}", ran.stdout);
    assert!(ran.stdout.is_empty(), "{}", ran.stdout);
    assert!(
        ran.stderr.contains("git config --global user.email"),
        "{}",
        ran.stderr
    );
    assert_eq!(env.snapshot(), before);
}

#[test]
fn a_foreign_hook_refuses_with_the_exec_line_and_is_left_alone() {
    let env = Env::new();
    let mine = b"#!/bin/sh\necho mine\n";
    let hook = env.repo.write(".git/hooks/pre-push", mine);
    let ran = env.jtac(&["init"]);
    assert_eq!(ran.code, 2, "{}", ran.stdout);
    let line = format!("exec '{}' hook pre-push \"$@\"", exe());
    assert!(ran.stderr.contains(&line), "{}", ran.stderr);
    assert!(ran.stderr.contains("negotiation: "), "{}", ran.stderr);
    assert_eq!(std::fs::read(&hook).unwrap(), mine);
    assert!(
        !env.repo.path().join(".jtac").exists(),
        "a refusal writes nothing"
    );
    let integrated = format!("#!/bin/sh\necho mine\n{line}\n");
    std::fs::write(&hook, &integrated).unwrap();
    executable(&hook);
    let ran = env.jtac(&["init"]);
    assert_eq!(ran.code, 2, "the line alone proves nothing: {}", ran.stdout);
    let ran = env.init(&["--keep-hook"]);
    assert!(
        ran.stdout.contains("kept pre-push hook") && ran.stdout.contains("yours, unread"),
        "{}",
        ran.stdout
    );
    assert_eq!(std::fs::read_to_string(&hook).unwrap(), integrated);
    let bare = Repo::bare(&[]);
    let url = slashes(bare.path());
    env.git(&["remote", "set-url", "origin", &url]);
    env.repo.write("a.txt", b"a\n");
    env.repo.commit("no root");
    let out = env.run("git", &["push", "origin", "main"]);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(!out.status.success(), "{stderr}");
    assert!(stderr.contains("has no root"), "{stderr}");
}

#[cfg(unix)]
fn executable(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
}

#[cfg(not(unix))]
fn executable(_: &Path) {}

/// Git skips a hook it cannot execute; init treats the right bytes with the wrong mode as
/// a step to do, on the first run and on any later one.
#[cfg(unix)]
#[test]
fn a_hook_git_cannot_execute_is_made_executable() {
    use std::os::unix::fs::PermissionsExt;
    let env = Env::new();
    env.init(&[]);
    let hook = env.repo.path().join(".git").join("hooks").join("pre-push");
    for (n, mode) in [0o644, 0o641, 0o654].into_iter().enumerate() {
        std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(mode)).unwrap();
        let ran = env.init(&[]);
        assert!(
            ran.stdout.contains("wrote pre-push hook") && ran.stdout.contains("made executable"),
            "{mode:o}: {}",
            ran.stdout
        );
        let now = std::fs::metadata(&hook).unwrap().permissions().mode();
        assert_eq!(now & 0o111, 0o111, "{now:o}");
        assert!(
            env.objects().len() >= 2,
            "{n}: the chmod is a change the lever records"
        );
    }
}

/// Git runs a hook through a symlink, so `--keep-hook` keeps one it can run and refuses one it
/// cannot: a link is never jtac's own and is never rewritten in place.
#[cfg(unix)]
#[test]
fn a_symlinked_hook_is_kept_only_when_git_can_run_it() {
    let env = Env::new();
    let target = env.repo.path().join("my-hook");
    std::fs::write(&target, b"#!/bin/sh\nexit 0\n").unwrap();
    let hook = env.repo.path().join(".git").join("hooks").join("pre-push");
    std::os::unix::fs::symlink(&target, &hook).unwrap();
    let ran = env.jtac(&["init", "--keep-hook"]);
    assert_eq!(ran.code, 2, "not executable: {}", ran.stdout);
    assert!(ran.stderr.contains("negotiation: "), "{}", ran.stderr);
    executable(&target);
    let ran = env.jtac(&["init"]);
    assert_eq!(ran.code, 2, "without --keep-hook: {}", ran.stdout);
    let ran = env.init(&["--keep-hook"]);
    assert!(ran.stdout.contains("yours, unread"), "{}", ran.stdout);
    assert!(std::fs::symlink_metadata(&hook).unwrap().is_symlink());
}

/// `create_new` guards the file name only; the directory it sits in must be this
/// repository's own.
#[cfg(unix)]
#[test]
fn a_symlinked_jtac_directory_is_refused_and_nothing_lands_behind_it() {
    let env = Env::new();
    let outside = Repo::empty();
    std::os::unix::fs::symlink(outside.path(), env.repo.path().join(".jtac")).unwrap();
    let ran = env.jtac(&["init"]);
    assert_eq!(ran.code, 2, "{}{}", ran.stdout, ran.stderr);
    assert!(ran.stderr.contains("is a symlink"), "{}", ran.stderr);
    assert!(ran.stderr.contains("negotiation: "), "{}", ran.stderr);
    assert_eq!(std::fs::read_dir(outside.path()).unwrap().count(), 0);
    assert!(!env.key().exists(), "a refusal writes nothing");
}

#[test]
fn a_jtac_hook_naming_a_moved_binary_is_rewritten() {
    let env = Env::new();
    let hook = env.repo.write(
        ".git/hooks/pre-push",
        hook_text("/workspace/old/jtac").as_bytes(),
    );
    let ran = env.init(&[]);
    assert!(
        ran.stdout.contains("(it named a jtac binary that moved)"),
        "{}",
        ran.stdout
    );
    assert_eq!(std::fs::read_to_string(&hook).unwrap(), hook_text(&exe()));
}

#[test]
fn core_hooks_path_is_where_the_hook_lands() {
    let env = Env::new();
    env.git(&["config", "core.hooksPath", "myhooks"]);
    env.init(&[]);
    let hook = env.repo.path().join("myhooks").join("pre-push");
    assert_eq!(std::fs::read_to_string(hook).unwrap(), hook_text(&exe()));
    assert!(!env.repo.path().join(".git/hooks/pre-push").exists());
}

#[test]
fn no_remote_notes_the_refspec_and_several_without_origin_refuse() {
    let env = Env::bare();
    let ran = env.init(&[]);
    assert!(
        ran.stdout
            .contains("note: no remote: run `jtac init` again after `git remote add origin <url>`"),
        "{}",
        ran.stdout
    );
    assert!(!ran.stdout.contains("fetch refspec"), "{}", ran.stdout);
    assert!(
        ran.stdout.contains("jtac init: 5 written, 0 kept"),
        "{}",
        ran.stdout
    );
    env.git(&["remote", "add", "up", "https://example.com/up.git"]);
    env.git(&["remote", "add", "down", "https://example.com/down.git"]);
    let ran = env.jtac(&["init"]);
    assert_eq!(ran.code, 2, "{}", ran.stdout);
    assert!(ran.stderr.contains("jtac init --remote "), "{}", ran.stderr);
    let ran = env.jtac(&["init", "--remote", "nope"]);
    assert_eq!(ran.code, 2, "{}", ran.stdout);
    assert!(ran.stderr.contains("git remote add nope"), "{}", ran.stderr);
    let ran = env.init(&["--remote", "up"]);
    assert!(
        ran.stdout
            .contains(&format!("  wrote fetch refspec {REFSPEC} on up\n")),
        "{}",
        ran.stdout
    );
    assert_eq!(
        env.git(&["config", "--get-all", "remote.up.fetch"])
            .lines()
            .filter(|l| *l == REFSPEC)
            .count(),
        1
    );
    assert_eq!(
        env.git(&["config", "--get-all", "remote.down.fetch"])
            .lines()
            .filter(|l| *l == REFSPEC)
            .count(),
        0
    );
}

#[test]
fn each_detected_tree_gets_the_default_policy_and_the_file_reads_back_to_its_hash() {
    let js = |runner, scripts: &[&str]| Detected {
        javascript: Some(Js {
            runner,
            scripts: scripts.iter().map(|s| (*s).to_owned()).collect(),
        }),
        ..Detected::default()
    };
    let pkg = r#"{"name":"example-app","scripts":{"test":"t","lint":"l","format:check":"f"}}"#;
    let all = ["format:check", "lint", "test"];
    let rows: Vec<(Vec<(&str, &str)>, Detected)> = vec![
        (
            vec![("Cargo.toml", "")],
            Detected {
                rust: true,
                ..Detected::default()
            },
        ),
        (
            vec![("go.mod", "")],
            Detected {
                go: true,
                ..Detected::default()
            },
        ),
        (
            vec![("pyproject.toml", "")],
            Detected {
                python: true,
                ..Detected::default()
            },
        ),
        (
            vec![("setup.py", "")],
            Detected {
                python: true,
                ..Detected::default()
            },
        ),
        (vec![("package.json", pkg)], js(Runner::Npm, &all)),
        (
            vec![("package.json", pkg), ("pnpm-lock.yaml", "")],
            js(Runner::Pnpm, &all),
        ),
        (
            vec![("package.json", pkg), ("yarn.lock", "")],
            js(Runner::Yarn, &all),
        ),
        (
            vec![("package.json", pkg), ("bun.lockb", "")],
            js(Runner::Bun, &all),
        ),
        (
            vec![("package.json", pkg), ("bun.lock", "")],
            js(Runner::Bun, &all),
        ),
        (vec![("README.md", "")], Detected::default()),
    ];
    let mut home = Env::bare().home;
    for (files, want) in rows {
        let env = Env {
            home,
            ..Env::bare()
        };
        for (name, text) in &files {
            env.repo.write(name, text.as_bytes());
        }
        let ran = env.init(&[]);
        let text = std::fs::read_to_string(env.repo.path().join(".jtac/policy.toml")).unwrap();
        let parsed = jtac_policy::parse(&text).unwrap();
        assert_eq!(
            policy_hash(&parsed).unwrap(),
            policy_hash(&default_policy(&want)).unwrap(),
            "{files:?}"
        );
        let none = ran.stdout.contains("note: no checks were detected");
        assert_eq!(
            none,
            want == Detected::default(),
            "{files:?}\n{}",
            ran.stdout
        );
        home = env.home;
    }
}

#[test]
fn json_carries_the_steps_the_notes_the_signer_and_the_policy() {
    let env = Env::new();
    let ran = env.init(&["--json", "--dry-run"]);
    let v: Value = serde_json::from_str(&ran.stdout).unwrap();
    assert_eq!(
        (
            v["v"].as_u64(),
            v["command"].as_str(),
            v["dry_run"].as_bool()
        ),
        (Some(1), Some("init"), Some(true))
    );
    let ran = env.init(&["--json"]);
    let v: Value = serde_json::from_str(&ran.stdout).unwrap();
    let steps: Vec<(&str, &str)> = v["steps"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| (s["step"].as_str().unwrap(), s["action"].as_str().unwrap()))
        .collect();
    assert_eq!(
        steps,
        [
            ("key", "wrote"),
            ("store", "wrote"),
            ("policy", "wrote"),
            ("hook", "wrote"),
            ("refspec", "wrote"),
            ("lever", "wrote")
        ]
    );
    assert!(
        v["steps"][3]["detail"]
            .as_str()
            .unwrap()
            .starts_with("pre-push hook "),
        "{v}"
    );
    assert!(
        v["notes"]
            .as_array()
            .unwrap()
            .iter()
            .any(|n| n.as_str().unwrap().contains("no checks were detected")),
        "{v}"
    );
    assert!(
        v["signers"]
            .as_str()
            .unwrap()
            .starts_with("dev@example.com ssh-ed25519 "),
        "{v}"
    );
    assert!(
        v["policy"].as_str().unwrap().starts_with("version = 1"),
        "{v}"
    );
    let ran = env.jtac(&["init", "--json", "--remote", "nope"]);
    assert_eq!(ran.code, 2);
    let err: Value = serde_json::from_str(&ran.stderr).unwrap();
    assert_eq!(err["error"]["kind"], "negotiation");
}

#[test]
fn outside_a_repository_init_refuses() {
    let env = Env {
        repo: Repo::empty(),
        home: Repo::empty(),
    };
    let ran = env.jtac(&["init"]);
    assert_eq!(ran.code, 2, "{}", ran.stdout);
    assert!(ran.stderr.starts_with("jtac: "), "{}", ran.stderr);
}
