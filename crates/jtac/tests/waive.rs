//! `jtac waive` against throwaway clones (DESIGN.md section 11.6): a rule waiver covers the
//! finding that blocks adopting jtac and the root stays covered; a commit waiver turns the
//! gate off for one commit; every refusal; an agent-session waiver is recorded, warned about,
//! and flagged by every reader; a waiver whose signature does not hold invalidates its root.

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use jtac_testkit::Repo;
use serde_json::Value;

fn me() -> String {
    env!("CARGO_BIN_EXE_jtac").replace('\\', "/")
}

fn policy() -> String {
    format!(
        "version = 1\n[checks.ok]\nrun = [\"{}\", \"--version\"]\n",
        me()
    )
}

/// A key under `.git/`, named in the local config; its allowed-signers line.
fn keyed(repo: &Repo) -> (PathBuf, String) {
    let key = repo.path().join(".git").join("key");
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
        .arg(&key)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let path = key.to_string_lossy().replace('\\', "/");
    repo.git(&["config", "jtac.signingkey", &path]);
    repo.git(&["config", "user.email", "dev@example.com"]);
    let pub_line = std::fs::read_to_string(key.with_extension("pub")).unwrap();
    let mut words = pub_line.split_whitespace();
    let line = format!(
        "dev@example.com {} {}\n",
        words.next().unwrap(),
        words.next().unwrap()
    );
    (key, line)
}

struct Ran {
    code: i32,
    stdout: String,
    stderr: String,
}

impl Ran {
    fn of(out: std::process::Output) -> Self {
        Self {
            code: out.status.code().unwrap_or(-1),
            stdout: String::from_utf8(out.stdout).unwrap(),
            stderr: String::from_utf8(out.stderr).unwrap(),
        }
    }
}

fn jtac(repo: &Repo, args: &[&str]) -> Ran {
    jtac_with(repo, args, &[])
}

fn jtac_with(repo: &Repo, args: &[&str], env: &[(&str, &str)]) -> Ran {
    Ran::of(repo.command_with(env!("CARGO_BIN_EXE_jtac"), args, env))
}

fn json(repo: &Repo, args: &[&str]) -> (i32, Value) {
    let ran = jtac(repo, args);
    assert!(ran.stderr.is_empty(), "{}", ran.stderr);
    (ran.code, serde_json::from_str(&ran.stdout).unwrap())
}

fn hook(repo: &Repo, stdin: &str) -> Ran {
    let mut child = Command::new(env!("CARGO_BIN_EXE_jtac"))
        .args([
            "hook",
            "pre-push",
            "origin",
            "https://example.com/example-app.git",
        ])
        .current_dir(repo.path())
        .env_clear()
        .envs(repo.env())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(stdin.as_bytes())
        .unwrap();
    Ran::of(child.wait_with_output().unwrap())
}

/// `base` on origin's `main` holds the policy and a trust file of comments alone; the clone
/// is keyed. The dev's line is what the first test adds: an edit under `.jtac/`, a major
/// (the first trust file itself is minor, decision 45; see `check.rs`).
struct Adopting {
    repo: Repo,
    key: PathBuf,
    signers: String,
    base: String,
}

fn adopting() -> Adopting {
    let origin = Repo::bare(&[]);
    let repo = Repo::clone_of(&origin);
    repo.write(".jtac/policy.toml", policy().as_bytes());
    repo.write(".jtac/signers", b"# who may sign\n");
    repo.write(".gitignore", b"*.profraw\n");
    repo.write("src/lib.rs", b"#[test]\nfn keeps() {}\nfn a() {}\n");
    let base = repo.commit("base");
    repo.git(&["push", "-q", "-u", "origin", "main"]);
    let (key, signers) = keyed(&repo);
    std::mem::forget(origin);
    Adopting {
        repo,
        key,
        signers,
        base,
    }
}

fn lines_with(text: &str, word: &str) -> usize {
    text.lines().filter(|l| l.contains(word)).count()
}

/// The trust case: editing `.jtac/signers` is a JT-PROT-001 major that blocks; a rule
/// waiver on the rule under `.jtac/` lets the verdict pass with the root `covered` and
/// `waivers` 1; `status`, `check`, `verify`, `audit`, the hook and `replay` all show it.
#[test]
fn a_rule_waiver_covers_the_adoption_finding_and_the_root_stays_covered() {
    let a = adopting();
    let repo = &a.repo;
    repo.write(".jtac/signers", a.signers.as_bytes());
    let tip = repo.commit("adopt");
    let ran = jtac(repo, &["check"]);
    assert_eq!(ran.code, 1, "{}{}", ran.stdout, ran.stderr);
    assert!(
        ran.stdout.contains("JT-PROT-001 major .jtac/signers"),
        "{}",
        ran.stdout
    );
    let before = jtac(repo, &["status"]);
    assert!(!before.stdout.contains("waiver"), "{}", before.stdout);

    let ran = jtac(
        repo,
        &[
            "waive",
            "--rule",
            "JT-PROT-001",
            "--path",
            ".jtac/",
            "--expires",
            "14d",
            "--reason",
            "adopting jtac: the policy and trust file land together",
        ],
    );
    assert_eq!(ran.code, 0, "{}{}", ran.stdout, ran.stderr);
    assert!(ran.stderr.is_empty(), "{}", ran.stderr);
    assert!(
        ran.stdout
            .starts_with("WAIVED JT-PROT-001 under .jtac/: lever sha256:"),
        "{}",
        ran.stdout
    );
    assert!(
        ran.stdout.contains("  signer  dev@example.com SHA256:"),
        "{}",
        ran.stdout
    );
    assert!(ran.stdout.contains("  expires 20"), "{}", ran.stdout);
    assert!(!ran.stdout.contains("agent"), "{}", ran.stdout);
    let (code, v) = json(repo, &["status", "--json"]);
    assert_eq!(code, 1);
    assert_eq!(v["waivers"][0]["scope"]["rule"], "JT-PROT-001");
    assert_eq!(v["waivers"][0]["principal"], "dev@example.com");
    assert!(v["waivers"][0]["agent"].is_null());
    let text = jtac(repo, &["status"]).stdout;
    assert!(
        text.contains("  waiver  JT-PROT-001 under .jtac/ by dev@example.com: adopting jtac"),
        "{text}"
    );

    let ran = jtac(repo, &["check"]);
    assert_eq!(ran.code, 0, "{}{}", ran.stdout, ran.stderr);
    assert!(ran.stdout.starts_with("CLEARED HOT"), "{}", ran.stdout);
    assert!(
        ran.stdout
            .contains("  waiver  JT-PROT-001 under .jtac/ by dev@example.com"),
        "{}",
        ran.stdout
    );
    assert!(
        ran.stdout
            .contains("  waived\n     JT-PROT-001 major .jtac/signers"),
        "{}",
        ran.stdout
    );
    assert!(!ran.stdout.contains("advisory"), "{}", ran.stdout);
    let (_, v) = json(repo, &["why", "--json"]);
    assert_eq!(v["state"], "covered");
    assert_eq!(v["waivers"].as_array().unwrap().len(), 1);
    assert_eq!(v["waived"][0]["rule"], "JT-PROT-001");
    assert!(v["advisory"].as_array().unwrap().is_empty());
    let root = v["root"].as_str().unwrap().to_owned();

    let (code, v) = json(repo, &["verify", "--json"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["pass"], true);
    assert_eq!(v["roots"], 1);
    assert_eq!(v["unknown_signers"], 0);
    assert_eq!(v["agent_waivers"], 0);
    assert_eq!(
        v["waivers"][0]["commit"].as_str().unwrap(),
        format!("sha1:{tip}")
    );
    assert_eq!(v["waivers"][0]["trust"], "trusted");
    assert_eq!(v["waivers"][0]["scope"]["path"], ".jtac/");
    let text = jtac(repo, &["verify"]).stdout;
    assert!(
        text.starts_with("VERIFIED HEAD: 2 commits, 1 covered"),
        "{text}"
    );
    assert_eq!(lines_with(&text, "  waiver  "), 1, "{text}");
    assert!(
        text.contains(&format!(
            "  waiver  {} JT-PROT-001 under .jtac/ by dev@example.com: adopting",
            &tip[..12]
        )),
        "{text}"
    );
    assert!(text.contains("(trusted)"), "{text}");

    let (_, v) = json(repo, &["audit", "--json"]);
    assert_eq!(v["totals"]["waivers"], 1);
    assert_eq!(v["totals"]["agent_waivers"], 0);
    assert_eq!(v["rows"][0]["state"], "covered");
    let text = jtac(repo, &["audit"]).stdout;
    assert_eq!(lines_with(&text, "  waiver  "), 1, "{text}");

    let ran = jtac(repo, &["replay", "HEAD"]);
    assert_eq!(ran.code, 0, "{}{}", ran.stdout, ran.stderr);
    assert!(ran.stdout.starts_with("REPLAYED"), "{}", ran.stdout);

    let line = format!("refs/heads/main {tip} refs/heads/main {}\n", a.base);
    let ran = hook(repo, &line);
    assert_eq!(ran.code, 0, "{}", ran.stderr);
    assert!(
        ran.stderr
            .contains("jtac: waiver: JT-PROT-001 under .jtac/ by dev@example.com"),
        "{}",
        ran.stderr
    );
    assert!(ran.stderr.contains("has a root, covered"), "{}", ran.stderr);
    let _ = (&a.key, root);
}

/// A commit waiver passes the verdict with state `declared_off` and keeps the reasons it
/// overrode; it names one commit, so the next commit diffs from the declared-off root and
/// carries no waiver.
#[test]
fn a_commit_waiver_turns_the_gate_off_for_one_commit_and_keeps_the_reasons() {
    let a = adopting();
    let repo = &a.repo;
    repo.write("src/lib.rs", b"fn a() {}\n");
    let tip = repo.commit("drop the test");
    let ran = jtac(repo, &["check"]);
    assert_eq!(ran.code, 1, "{}", ran.stdout);
    assert!(ran.stdout.contains("JT-TEST-001 blocker"), "{}", ran.stdout);

    let ran = jtac(
        repo,
        &[
            "waive",
            "HEAD",
            "--reason",
            "the feature and its test went together",
        ],
    );
    assert_eq!(ran.code, 0, "{}{}", ran.stdout, ran.stderr);
    assert!(
        ran.stdout
            .starts_with(&format!("WAIVED commit {tip}: lever sha256:")),
        "{}",
        ran.stdout
    );
    assert!(
        ran.stdout.contains("  expires none (covers one commit)"),
        "{}",
        ran.stdout
    );
    assert!(
        ran.stdout.contains("its root reads declared off"),
        "{}",
        ran.stdout
    );

    let text = jtac(repo, &["check", "--preview"]).stdout;
    assert!(
        text.starts_with(
            "CLEARED HOT (preview, nothing written): declared off by a commit waiver, 1 reasons \
             overridden; 2 of 2 checks pass"
        ),
        "{text}"
    );
    let (code, v) = json(repo, &["check", "--json"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["banner"], "CLEARED HOT");
    assert_eq!(v["state"], "declared_off");
    assert_eq!(v["reasons"][0]["rule"], "JT-TEST-001");
    assert_eq!(
        v["waivers"][0]["scope"]["commit"].as_str().unwrap(),
        format!("sha1:{tip}")
    );
    let text = jtac(repo, &["audit"]).stdout;
    assert!(text.contains("declared_off"), "{text}");
    assert_eq!(lines_with(&text, "  waiver  "), 1, "{text}");
    let line = format!("refs/heads/main {tip} refs/heads/main {}\n", a.base);
    let ran = hook(repo, &line);
    assert_eq!(ran.code, 0, "{}", ran.stderr);
    assert!(
        ran.stderr
            .contains("declared off: a waiver turned the gate off"),
        "{}",
        ran.stderr
    );
    assert!(
        ran.stderr
            .contains(&format!("jtac: waiver: commit {tip} by dev@example.com")),
        "{}",
        ran.stderr
    );

    repo.write("b.txt", b"b\n");
    repo.commit("next");
    let status = jtac(repo, &["status"]).stdout;
    assert!(!status.contains("  waiver  "), "{status}");
    let (code, v) = json(repo, &["check", "--json"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["state"], "covered");
    assert_eq!(v["base"]["from"], "nearest covered ancestor");
    assert!(v["waivers"].as_array().unwrap().is_empty(), "{v}");
    let ran = jtac(repo, &["waive", "HEAD", "--reason", "late"]);
    assert_eq!(ran.code, 0, "{}", ran.stderr);
    let (code, v) = json(repo, &["check", "--json"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["state"], "declared_off", "this run's state: {v}");
    assert_eq!(v["waivers"][0]["reason"], "late", "{v}");
    let notes = v["notes"].to_string();
    assert!(
        notes.contains("it records covered with 0 waivers, this run judged declared_off with 1"),
        "{v}"
    );
}

/// A control character in any word is refused before any other refusal could quote it, so no
/// refusal prints one; the flag at fault is named.
#[test]
fn a_control_character_in_any_word_of_a_waiver_is_refused_before_it_is_quoted() {
    let a = adopting();
    let repo = &a.repo;
    let rows: [(&[&str], &str); 3] = [
        (
            &[
                "waive",
                "--rule",
                "JT-TEST-001",
                "--path",
                "src\r\x1b[1A\x1b[2K",
                "--expires",
                "3d",
                "--reason",
                "x",
            ],
            "`--path` is blank or holds a control character",
        ),
        (
            &[
                "waive",
                "--rule",
                "JT-TEST-001\rJT-NOPE",
                "--path",
                "src",
                "--expires",
                "3d",
                "--reason",
                "x",
            ],
            "`--rule` is blank or holds a control character",
        ),
        (
            &["waive", "HEAD", "--expires", "3d", "--reason", "a\tb"],
            "`--reason` is blank or holds a control character",
        ),
    ];
    for (args, want) in rows {
        let ran = jtac(repo, args);
        assert_eq!(ran.code, 2, "{args:?}: {}{}", ran.stdout, ran.stderr);
        assert!(ran.stderr.contains(want), "{args:?}: {}", ran.stderr);
        assert_eq!(ran.stderr.lines().count(), 3, "{args:?}: {}", ran.stderr);
        assert!(
            !ran.stderr.chars().any(|c| c.is_control() && c != '\n'),
            "{args:?}: {:?}",
            ran.stderr
        );
    }
    assert!(!repo.path().join(".git/jtac/levers").exists());
}

/// Every refusal is three lines with the next command, and the lever store stays empty.
#[test]
fn a_waiver_with_no_scope_a_bad_expiry_a_bad_path_a_blank_reason_or_a_bad_rev_is_refused() {
    let a = adopting();
    let repo = &a.repo;
    let rows: [(&[&str], &str); 9] = [
        (&["waive", "--reason", "x"], "needs a scope"),
        (
            &["waive", "--rule", "JT-SUP-001", "--reason", "x"],
            "needs a scope",
        ),
        (
            &[
                "waive",
                "--rule",
                "JT-SUP-001",
                "--path",
                "src",
                "--reason",
                "x",
            ],
            "needs `--expires`",
        ),
        (
            &[
                "waive",
                "--rule",
                "JT-SUP-001",
                "--path",
                "src",
                "--expires",
                "31d",
                "--reason",
                "x",
            ],
            "`--expires 31d` is not a number of days from 1 to 30",
        ),
        (
            &[
                "waive",
                "--rule",
                "JT-SUP-001",
                "--path",
                "src",
                "--expires",
                "7",
                "--reason",
                "x",
            ],
            "`--expires 7` is not",
        ),
        (
            &[
                "waive",
                "--rule",
                "JT-SUP-001",
                "--path",
                "src/../x",
                "--expires",
                "7d",
                "--reason",
                "x",
            ],
            "`--path src/../x` is not a clean",
        ),
        (
            &[
                "waive",
                "HEAD~1",
                "--expires",
                "7d",
                "--reason",
                "base only",
            ],
            "a commit waiver takes no `--expires`",
        ),
        (&["waive", "HEAD", "--reason", "  "], "`--reason` is blank"),
        (
            &["waive", "HEAD", "--reason", "x\r  waiver  forged"],
            "`--reason` is blank or holds a control character",
        ),
    ];
    for (args, want) in rows {
        let ran = jtac(repo, args);
        assert_eq!(
            (ran.code, ran.stdout.as_str()),
            (2, ""),
            "{args:?}: {}",
            ran.stderr
        );
        assert!(ran.stderr.contains(want), "{args:?}: {}", ran.stderr);
        assert_eq!(ran.stderr.lines().count(), 3, "{args:?}: {}", ran.stderr);
        assert!(
            ran.stderr.contains("\nnegotiation: Run `jtac waive"),
            "{args:?}: {}",
            ran.stderr
        );
        if args.contains(&"--path") {
            assert!(
                ran.stderr.contains("--rule JT-SUP-001"),
                "{args:?}: {}",
                ran.stderr
            );
        }
    }
    let ran = jtac(
        repo,
        &[
            "waive",
            "HEAD~1",
            "--expires",
            "7d",
            "--reason",
            "base only",
        ],
    );
    assert!(
        ran.stderr
            .contains("`jtac waive 'HEAD~1' --reason 'base only'`"),
        "{}",
        ran.stderr
    );
    let ran = jtac(repo, &["waive", "nope", "--reason", "x"]);
    assert_eq!(ran.code, 2, "{}", ran.stderr);
    assert!(
        ran.stderr.contains("`nope` does not name a commit"),
        "{}",
        ran.stderr
    );
    assert!(ran.stderr.contains("`jtac waive`"), "{}", ran.stderr);
    let ran = jtac(repo, &["waive", "HEAD", "--reason", "x", "--json"]);
    assert_eq!(ran.code, 0, "{}", ran.stderr);
    let v: Value = serde_json::from_str(&ran.stdout).unwrap();
    assert_eq!(v["command"], "waive");
    assert!(v["expires"].is_null());
    assert_eq!(v["signer"]["principal"], "dev@example.com");
    assert!(
        !repo
            .path()
            .join(".git/jtac/levers")
            .read_dir()
            .unwrap()
            .next()
            .is_none()
    );
    assert_eq!(
        repo.path()
            .join(".git/jtac/levers")
            .read_dir()
            .unwrap()
            .count(),
        1
    );
}

/// Under an agent marker the waiver is written, the command warns on stderr with the file
/// that withdraws it, the lever node carries the marker, and `status`, `check`, `verify`,
/// `audit` and the hook each say so.
#[test]
fn an_agent_session_waiver_is_recorded_warned_about_and_flagged_by_every_reader() {
    let a = adopting();
    let repo = &a.repo;
    repo.write(".jtac/signers", a.signers.as_bytes());
    let tip = repo.commit("adopt");
    let ran = jtac_with(
        repo,
        &[
            "waive",
            "--rule",
            "JT-PROT-001",
            "--path",
            ".jtac",
            "--expires",
            "30d",
            "--reason",
            "r",
            "--json",
        ],
        &[("CLAUDECODE", "1")],
    );
    assert_eq!(ran.code, 0, "{}", ran.stderr);
    let v: Value = serde_json::from_str(&ran.stdout).unwrap();
    assert_eq!(v["agent"], "CLAUDECODE");
    assert!(
        ran.stderr
            .starts_with("jtac: this waiver was written under an agent session (CLAUDECODE)."),
        "{}",
        ran.stderr
    );
    assert_eq!(ran.stderr.lines().count(), 3, "{}", ran.stderr);
    let lever = v["lever"]
        .as_str()
        .unwrap()
        .trim_start_matches("sha256:")
        .to_owned();
    let sig = format!(".git/jtac/levers/{lever}.sig");
    assert!(ran.stderr.contains("`rm "), "{}", ran.stderr);
    assert!(ran.stderr.contains(&lever), "{}", ran.stderr);
    assert!(repo.path().join(&sig).is_file());
    let text = jtac_with(
        repo,
        &["waive", "HEAD", "--reason", "r"],
        &[("CODEX_SANDBOX", "1")],
    );
    assert!(
        text.stdout
            .contains("  agent   written under an agent session (CODEX_SANDBOX)"),
        "{}",
        text.stdout
    );
    let withdraw = text
        .stdout
        .lines()
        .find(|l| l.starts_with("WAIVED"))
        .unwrap();
    let commit_lever = withdraw.rsplit("sha256:").next().unwrap().trim();
    std::fs::remove_file(
        repo.path()
            .join(format!(".git/jtac/levers/{commit_lever}.sig")),
    )
    .unwrap();

    let status = jtac(repo, &["status"]).stdout;
    assert!(
        status.contains("by dev@example.com under agent session CLAUDECODE: r"),
        "{status}"
    );
    let ran = jtac(repo, &["check"]);
    assert_eq!(ran.code, 0, "{}{}", ran.stdout, ran.stderr);
    assert!(
        ran.stdout.contains("under agent session CLAUDECODE"),
        "{}",
        ran.stdout
    );
    let (_, v) = json(repo, &["why", "--json"]);
    assert_eq!(v["state"], "covered");
    assert_eq!(v["waivers"][0]["agent"], "CLAUDECODE");
    let (code, v) = json(repo, &["verify", "--json"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["agent_waivers"], 1);
    assert_eq!(v["waivers"][0]["agent"], "CLAUDECODE");
    let text = jtac(repo, &["verify"]).stdout;
    assert!(text.contains("under agent session CLAUDECODE"), "{text}");
    assert!(
        text.contains("  note    ")
            && text.contains("applies a waiver written under an agent session (CLAUDECODE)"),
        "{text}"
    );
    let (_, v) = json(repo, &["audit", "--json"]);
    assert_eq!(v["totals"]["agent_waivers"], 1);
    let line = format!("refs/heads/main {tip} refs/heads/main {}\n", a.base);
    let ran = hook(repo, &line);
    assert_eq!(ran.code, 0, "{}", ran.stderr);
    assert!(ran.stderr.contains("jtac: waiver: JT-PROT-001 under .jtac by dev@example.com under agent session CLAUDECODE: r"), "{}", ran.stderr);
}

fn git_stdin(repo: &Repo, args: &[&str], stdin: &[u8]) -> String {
    let mut child = Command::new("git")
        .args(args)
        .current_dir(repo.path())
        .env_clear()
        .envs(repo.env())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.as_mut().unwrap().write_all(stdin).unwrap();
    let out = child.wait_with_output().unwrap();
    assert!(out.status.success(), "git {args:?}");
    String::from_utf8(out.stdout).unwrap().trim().to_owned()
}

/// Replace `commit`'s root ref with `json`, freshly signed by `key`.
fn replace_root(repo: &Repo, key: &Path, commit: &str, json: &str) {
    let json_path = repo.write(".git/forged/root.json", json.as_bytes());
    let sig_path = json_path.with_extension("json.sig");
    let _ = std::fs::remove_file(&sig_path);
    let out = Command::new("ssh-keygen")
        .args(["-Y", "sign", "-n", "jtac-root-v1", "-P", "", "-f"])
        .arg(key)
        .arg(&json_path)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let sig = std::fs::read_to_string(sig_path).unwrap();
    let sig_path = repo.write(".git/forged/root.sig", sig.as_bytes());
    let j = repo.git(&["hash-object", "-w", "--", json_path.to_str().unwrap()]);
    let s = repo.git(&["hash-object", "-w", "--", sig_path.to_str().unwrap()]);
    let listing = format!("100644 blob {j}\troot.json\n100644 blob {s}\troot.sig\n");
    let tree = git_stdin(repo, &["mktree"], listing.as_bytes());
    let forged = repo.git(&["commit-tree", &tree, "-m", "forged"]);
    let name = format!("refs/jtac/roots/{}/{commit}", &commit[..2]);
    repo.git(&["update-ref", &name, &forged]);
}

/// A stored waiver whose signature does not hold is never applied, so `check` cannot sign a
/// root `verify` would reject; a root that carries one anyway is invalid; with no trust file
/// both the root's and the waiver's signers are unknown and the root counts once.
#[test]
fn a_waiver_whose_signature_does_not_hold_invalidates_the_root_that_carries_it() {
    let a = adopting();
    let repo = &a.repo;
    repo.write("src/lib.rs", b"fn a() {}\n");
    let tip = repo.commit("drop the test");
    let ran = jtac(repo, &["waive", "HEAD", "--reason", "r"]);
    assert_eq!(ran.code, 0, "{}", ran.stderr);
    let sig_file = repo
        .path()
        .join(".git/jtac/levers")
        .read_dir()
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    let good = std::fs::read_to_string(&sig_file).unwrap();
    std::fs::write(&sig_file, good.replacen("AAAA", "AAAB", 1)).unwrap();
    let ran = jtac(repo, &["check"]);
    assert_eq!(ran.code, 1, "{}{}", ran.stdout, ran.stderr);
    assert!(
        ran.stdout
            .contains("is not applied: its signature does not hold"),
        "{}",
        ran.stdout
    );
    assert!(ran.stdout.contains("JT-TEST-001 blocker"), "{}", ran.stdout);
    let status = jtac(repo, &["status"]).stdout;
    assert!(
        !status.contains("  waiver  ") && status.contains("is not applied"),
        "{status}"
    );
    let other = repo.path().join(".git").join("other");
    let out = Command::new("ssh-keygen")
        .args(["-q", "-t", "ed25519", "-N", "", "-f"])
        .arg(&other)
        .output()
        .unwrap();
    assert!(out.status.success());
    let node_hex = sig_file.file_stem().unwrap().to_str().unwrap().to_owned();
    let node_path = repo.path().join(".git/jtac/objects").join(node_hex);
    let out = Command::new("ssh-keygen")
        .args(["-Y", "sign", "-n", "jtac-lever-v1", "-P", "", "-f"])
        .arg(&other)
        .arg(&node_path)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let other_key = node_path.with_extension("sig");
    std::fs::write(&sig_file, std::fs::read(&other_key).unwrap()).unwrap();
    std::fs::remove_file(other_key).unwrap();
    let ran = jtac(repo, &["check"]);
    assert_eq!(ran.code, 1, "{}{}", ran.stdout, ran.stderr);
    assert!(
        ran.stdout
            .contains("is not applied: its signature does not hold"),
        "{}",
        ran.stdout
    );
    repo.git(&["config", "gpg.ssh.program", "/nonexistent/ssh-keygen"]);
    let ran = jtac(repo, &["check", "--preview"]);
    assert_eq!(ran.code, 2, "{}{}", ran.stdout, ran.stderr);
    assert!(
        ran.stderr
            .starts_with("jtac: cannot check a stored waiver's signature"),
        "{}",
        ran.stderr
    );
    assert!(!ran.stderr.contains("push"), "{}", ran.stderr);
    repo.git(&["config", "--unset", "gpg.ssh.program"]);
    std::fs::write(&sig_file, good).unwrap();
    let ran = jtac(repo, &["check"]);
    assert_eq!(ran.code, 0, "{}{}", ran.stdout, ran.stderr);
    let (code, v) = json(repo, &["verify", "--json"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["unknown_signers"], 1, "{v}");
    assert_eq!(v["waivers"][0]["trust"], "unknown");
    let (_, v) = json(repo, &["audit", "--json"]);
    assert_eq!(v["totals"]["unknown_signers"], 1, "{v}");

    let name = format!("refs/jtac/roots/{}/{tip}", &tip[..2]);
    let out = repo.command("git", &["cat-file", "blob", &format!("{name}:root.json")]);
    let mut root: Value = serde_json::from_slice(&out.stdout).unwrap();
    let sig = root["levers"][0]["sig"].as_str().unwrap().to_owned();
    root["levers"][0]["sig"] = Value::String(sig.replacen("AAAA", "AAAB", 1));
    assert_ne!(root["levers"][0]["sig"], sig);
    let json_text = jtac_canonical(&root);
    replace_root(repo, &a.key, &tip, &json_text);
    let ran = jtac(repo, &["verify"]);
    assert_eq!(ran.code, 1, "{}{}", ran.stdout, ran.stderr);
    assert!(ran.stdout.starts_with("INVALID HEAD"), "{}", ran.stdout);
    assert!(
        ran.stdout
            .contains("its waiver 0's signature does not hold"),
        "{}",
        ran.stdout
    );
    assert!(!ran.stdout.contains("  waiver  "), "{}", ran.stdout);
}

/// RFC 8785 for a `serde_json::Value`: sorted keys, no whitespace, which `Value`'s default
/// `to_string` gives once the map is sorted (the crate is built with `preserve_order` off).
fn jtac_canonical(v: &Value) -> String {
    v.to_string()
}

/// The CI waiver (DESIGN.md section 15, decision 48): a commit pushed around CI has no CI
/// root; `waive --ci` records why, `check --push` writes the root that carries it and sends
/// it, the verdict stays `covered`, and a second clone's `audit` reads the CI gate declared
/// off with the reason beside it. `--ci` with another flag, or after a root stands, is refused.
#[test]
fn a_ci_waiver_rides_in_the_root_on_its_commit_and_audit_reads_the_ci_gate_declared_off() {
    let a = adopting();
    let repo = &a.repo;
    repo.write(".jtac/signers", a.signers.as_bytes());
    repo.commit("adopt");
    let ran = jtac(
        repo,
        &[
            "waive",
            "--rule",
            "JT-PROT-001",
            "--path",
            ".jtac/",
            "--expires",
            "14d",
            "--reason",
            "adopting",
        ],
    );
    assert_eq!(ran.code, 0, "{}{}", ran.stdout, ran.stderr);
    let ran = jtac(repo, &["check", "--push", "origin"]);
    assert_eq!(ran.code, 0, "{}{}", ran.stdout, ran.stderr);
    repo.write("src/b.rs", b"fn b() {}\n");
    let merge = repo.commit("merged around CI");
    repo.git(&["push", "-q", "origin", "main"]);

    let ran = jtac(
        repo,
        &["waive", "HEAD", "--ci", "--expires", "3d", "--reason", "x"],
    );
    assert_ne!(ran.code, 0);
    assert!(
        ran.stderr.contains("`--ci` takes a commit alone"),
        "{}",
        ran.stderr
    );
    let ran = jtac(repo, &["waive", "HEAD", "--ci", "--reason", "runners out"]);
    assert_eq!(ran.code, 0, "{}{}", ran.stdout, ran.stderr);
    assert!(
        ran.stdout
            .starts_with(&format!("WAIVED ci gate on commit {merge}: lever sha256:")),
        "{}",
        ran.stdout
    );
    assert!(
        ran.stdout.contains("  expires none (covers one commit)")
            && ran.stdout.contains("jtac check --push <remote>"),
        "{}",
        ran.stdout
    );

    let (code, v) = json(repo, &["check", "--preview", "--json"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["state"], "covered");
    assert_eq!(v["waivers"][0]["scope"]["gate"], "ci");
    let ran = jtac(repo, &["check", "--push", "origin", "--json"]);
    assert_eq!(ran.code, 0, "{}{}", ran.stdout, ran.stderr);
    let v: Value = serde_json::from_str(&ran.stdout).unwrap();
    assert_eq!(v["state"], "covered");
    assert_eq!(v["waivers"][0]["reason"], "runners out");
    let root_ref = format!("refs/jtac/roots/{}/{merge}", &merge[..2]);
    assert!(
        v["notes"]
            .to_string()
            .contains(&format!("origin holds {root_ref}")),
        "{v}"
    );
    let ran = jtac(repo, &["replay", "HEAD"]);
    assert_eq!(ran.code, 0, "{}{}", ran.stdout, ran.stderr);
    let ran = jtac(repo, &["waive", "HEAD", "--ci", "--reason", "again"]);
    assert_ne!(ran.code, 0);
    assert!(
        ran.stderr.contains("already has a root")
            && ran
                .stderr
                .contains(&format!("git update-ref -d {root_ref}")),
        "{}",
        ran.stderr
    );

    let origin = Repo::empty();
    origin.git(&[
        "clone",
        "-q",
        "--",
        &repo.git(&["remote", "get-url", "origin"]),
        ".",
    ]);
    origin.git(&["fetch", "-q", "origin", "+refs/jtac/*:refs/jtac/*"]);
    let (code, v) = json(&origin, &["audit", "main", "--json"]);
    assert_eq!(code, 0, "{v}");
    let row = v["rows"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["commit"] == format!("sha1:{merge}"))
        .unwrap();
    assert_eq!(row["state"], "covered");
    assert_eq!(row["ci"], "declared_off");
    assert_eq!(row["trust"], "trusted");
    let waiver = v["waivers"]
        .as_array()
        .unwrap()
        .iter()
        .find(|w| w["scope"]["gate"] == "ci")
        .unwrap();
    assert_eq!(waiver["reason"], "runners out");
    assert_eq!(waiver["trust"], "trusted");
    let text = jtac(&origin, &["audit", "main"]).stdout;
    assert!(
        text.contains("ci declared_off: a waiver this root carries")
            && text.contains(&format!(
                "ci gate on commit {merge} by dev@example.com: runners out (trusted)"
            )),
        "{text}"
    );
    let (code, v) = json(&origin, &["verify", "main", "--json"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["unknown_signers"], 0);
    assert_eq!(v["roots"], 2);
}
