//! `jtac ci` on scripted pull requests (DESIGN.md section 15, M15): one verdict from the merge
//! base with trust from the base branch, the chain's trusted waivers offered and the withheld
//! ones named with why, HEAD's root verified and compared, and each failure in three parts.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use jtac_core::{Covers, FromHead, GitOid, Root, State};
use jtac_testkit::Repo;
use serde_json::Value;

fn me() -> String {
    env!("CARGO_BIN_EXE_jtac").replace('\\', "/")
}

fn with_policy(repo: &Repo) {
    let policy = format!(
        "version = 1\n[checks.ok]\nrun = [\"{}\", \"--version\"]\n",
        me()
    );
    repo.write(".jtac/policy.toml", policy.as_bytes());
    repo.write(".gitignore", b"*.profraw\n");
}

fn key(repo: &Repo, name: &str, email: &str) -> PathBuf {
    let key = repo.path().join(".git").join(name);
    let out = Command::new("ssh-keygen")
        .args(["-q", "-t", "ed25519", "-N", "", "-C", email, "-f"])
        .arg(&key)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    key
}

fn fingerprint(key: &Path) -> String {
    let out = Command::new("ssh-keygen")
        .arg("-lf")
        .arg(key.with_extension("pub"))
        .output()
        .unwrap();
    String::from_utf8(out.stdout)
        .unwrap()
        .split_whitespace()
        .nth(1)
        .unwrap()
        .to_owned()
}

fn signers_line(key: &Path, email: &str) -> String {
    let pubkey = std::fs::read_to_string(key.with_extension("pub")).unwrap();
    let pubkey: Vec<&str> = pubkey.split_whitespace().take(2).collect();
    format!("{email} {}\n", pubkey.join(" "))
}

struct Ran {
    code: i32,
    stdout: String,
    stderr: String,
}

fn jtac(repo: &Repo, args: &[&str]) -> Ran {
    let out = repo.command(env!("CARGO_BIN_EXE_jtac"), args);
    Ran {
        code: out.status.code().unwrap_or(-1),
        stdout: String::from_utf8(out.stdout).unwrap(),
        stderr: String::from_utf8(out.stderr).unwrap(),
    }
}

fn ok(repo: &Repo, args: &[&str]) {
    let ran = jtac(repo, args);
    assert_eq!(ran.code, 0, "{args:?}: {}{}", ran.stdout, ran.stderr);
}

fn json(repo: &Repo, args: &[&str]) -> (i32, Value) {
    let ran = jtac(repo, args);
    assert!(ran.stderr.is_empty(), "{}", ran.stderr);
    (ran.code, serde_json::from_str(&ran.stdout).unwrap())
}

fn ci(repo: &Repo) -> (i32, Value) {
    json(repo, &["ci", "--base", "main", "--json"])
}

fn root_ref(commit: &str) -> String {
    format!("refs/jtac/roots/{}/{commit}", &commit[..2])
}

fn root_json(repo: &Repo, commit: &str) -> String {
    let out = repo.command(
        "git",
        &[
            "cat-file",
            "blob",
            &format!("{}:root.json", root_ref(commit)),
        ],
    );
    assert!(out.status.success(), "{commit}");
    String::from_utf8(out.stdout).unwrap()
}

fn read_root(repo: &Repo, commit: &str) -> Root {
    serde_json::from_str(&root_json(repo, commit)).unwrap()
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
    std::io::Write::write_all(child.stdin.as_mut().unwrap(), stdin).unwrap();
    let out = child.wait_with_output().unwrap();
    assert!(out.status.success(), "git {args:?}");
    String::from_utf8(out.stdout).unwrap().trim().to_owned()
}

/// Replace `commit`'s root ref with `root`, canonical and freshly signed by `key`.
fn put(repo: &Repo, key: &Path, commit: &str, root: &Root) {
    let json = String::from_utf8(root.canonical_bytes().unwrap()).unwrap();
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
    repo.git(&["update-ref", &root_ref(commit), &forged]);
}

fn g(hex: &str) -> GitOid {
    format!("sha1:{hex}").parse().unwrap()
}

/// Whom `main`'s signers file names.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Trusts {
    Dev,
    Other,
    NoFile,
}

/// `main` with a policy, one test, a signers file per `trusts`, and a root; `feature` is
/// checked out at `main`, the developer key configured for `jtac check`.
struct Pr {
    repo: Repo,
    dev: PathBuf,
    other: PathBuf,
    main: String,
}

fn pr(trusts: Trusts) -> Pr {
    let repo = Repo::init(&[]);
    with_policy(&repo);
    repo.write("src/lib.rs", b"#[test]\nfn keeps() {}\nfn a() {}\n");
    let dev = key(&repo, "dev", "dev@example.com");
    let other = key(&repo, "other", "other@example.com");
    match trusts {
        Trusts::Dev => repo.write(
            ".jtac/signers",
            signers_line(&dev, "dev@example.com").as_bytes(),
        ),
        Trusts::Other => repo.write(
            ".jtac/signers",
            signers_line(&other, "other@example.com").as_bytes(),
        ),
        Trusts::NoFile => PathBuf::new(),
    };
    let path = dev.to_string_lossy().replace('\\', "/");
    repo.git(&["config", "jtac.signingkey", &path]);
    repo.git(&["config", "user.email", "dev@example.com"]);
    let main = repo.commit("main");
    ok(&repo, &["check"]);
    repo.git(&["switch", "-q", "-c", "feature"]);
    Pr {
        repo,
        dev,
        other,
        main,
    }
}

impl Pr {
    fn add(&self, path: &str, text: &str) -> String {
        self.repo.write(path, text.as_bytes());
        self.repo.commit(path)
    }

    fn remove_test(&self) -> String {
        self.repo.write("src/lib.rs", b"fn a() {}\n");
        self.repo.commit("remove the test")
    }

    fn state_json(&self) -> Vec<u8> {
        std::fs::read(self.repo.path().join(".git/jtac/state.json")).unwrap()
    }
}

fn rules(v: &Value) -> Vec<&str> {
    v["verdict"]["reasons"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|r| r["rule"].as_str())
        .collect()
}

fn three_parts(v: &Value) {
    for r in v["reasons"].as_array().unwrap() {
        for part in ["what", "why", "how"] {
            assert!(
                r[part].as_str().is_some_and(|s| s.len() > 20),
                "{part}: {r}"
            );
        }
    }
}

#[test]
fn a_clean_pull_request_with_an_agreeing_trusted_head_root_passes_and_writes_no_root_or_state() {
    let p = pr(Trusts::Dev);
    let tip = p.add("a.txt", "a\n");
    ok(&p.repo, &["check"]);
    let before = p.state_json();
    let (code, v) = ci(&p.repo);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["v"], 1);
    assert_eq!(v["command"], "ci");
    assert_eq!(v["pass"], true);
    assert_eq!(v["adoption"], false);
    assert_eq!(v["policy_from"], "head");
    assert_eq!(v["waivable"], 1);
    assert_eq!(v["signers"], "main:.jtac/signers");
    assert_eq!(v["verdict"]["command"], "ci");
    assert_eq!(v["verdict"]["banner"], "CLEARED HOT");
    assert_eq!(v["verdict"]["base"]["commit"], p.main);
    assert_eq!(v["verdict"]["commit"], format!("sha1:{tip}"));
    assert!(v["verdict"]["root"].is_null(), "{v}");
    assert_eq!(v["head_root"]["compared"], "agreed");
    assert_eq!(v["head_root"]["trust"], "trusted");
    assert_eq!(v["head_root"]["signer"]["principal"], "dev@example.com");
    assert_eq!(v["head_root"]["recorded"], v["head_root"]["run"]);
    assert_eq!(v["reasons"], serde_json::json!([]));
    assert_eq!(p.state_json(), before, "ci wrote state.json");
    let ran = jtac(&p.repo, &["ci", "--base", "main"]);
    assert_eq!(ran.code, 0, "{}", ran.stderr);
    assert!(
        ran.stdout
            .starts_with("CLEARED HOT: 2 of 2 checks pass, nothing blocks; head root agreed\n"),
        "{}",
        ran.stdout
    );
    assert!(
        ran.stdout.contains("  signers main:.jtac/signers\n"),
        "{}",
        ran.stdout
    );
    assert!(
        ran.stdout.contains("  head    root sha256:"),
        "{}",
        ran.stdout
    );
    assert!(ran.stdout.contains("(trusted): agreed"), "{}", ran.stdout);
    assert!(!ran.stdout.contains("none written"), "{}", ran.stdout);
}

#[test]
fn a_false_head_root_fails_with_three_parts_and_both_sides_counts() {
    let p = pr(Trusts::Dev);
    let tip = p.add("a.txt", "a\n");
    ok(&p.repo, &["check"]);
    let mut forged = read_root(&p.repo, &tip);
    forged.counts.findings.minor += 1;
    put(&p.repo, &p.dev, &tip, &forged);
    let (code, v) = ci(&p.repo);
    assert_eq!(code, 1, "{v}");
    assert_eq!(v["verdict"]["pass"], true);
    assert_eq!(v["head_root"]["compared"], "disagreed");
    assert_eq!(v["head_root"]["recorded"]["counts"]["findings"]["minor"], 1);
    assert_eq!(v["head_root"]["run"]["counts"]["findings"]["minor"], 0);
    three_parts(&v);
    let what = v["reasons"][0]["what"].as_str().unwrap();
    assert!(
        what.contains("records covered with checks 2/2 pass, findings 0 blocker 0 major 1 minor"),
        "{what}"
    );
    assert!(
        what.contains("reaches covered with checks 2/2 pass, findings 0 blocker 0 major 0 minor"),
        "{what}"
    );
    assert!(
        v["reasons"][0]["how"]
            .as_str()
            .unwrap()
            .contains(&format!("git update-ref -d {}", root_ref(&tip))),
        "{v}"
    );
    let ran = jtac(&p.repo, &["ci", "--base", "main"]);
    assert_eq!(ran.code, 1);
    assert!(ran.stdout.starts_with("ABORT: "), "{}", ran.stdout);
    assert!(
        ran.stdout.contains("; head root disagreed\n"),
        "{}",
        ran.stdout
    );
    assert!(
        ran.stdout.contains("          recorded covered with"),
        "{}",
        ran.stdout
    );
    assert!(
        ran.stdout.contains("          run      covered with"),
        "{}",
        ran.stdout
    );
    assert!(
        ran.stdout.contains("  1. HEAD's root records"),
        "{}",
        ran.stdout
    );
    assert!(
        ran.stdout.contains("     why  The root's verdict"),
        "{}",
        ran.stdout
    );
    assert!(
        ran.stdout.contains("     fix  Run `jtac replay"),
        "{}",
        ran.stdout
    );
    let mut off = read_root(&p.repo, &tip);
    off.state = State::DeclaredOff;
    put(&p.repo, &p.dev, &tip, &off);
    let (code, v) = ci(&p.repo);
    assert_eq!(code, 1, "{v}");
    assert_eq!(v["head_root"]["compared"], "invalid");
    assert!(v["head_root"]["root"].is_null(), "{v}");
    assert!(
        v["reasons"][0]["what"]
            .as_str()
            .unwrap()
            .starts_with("HEAD's root does not count: "),
        "{v}"
    );
    three_parts(&v);
}

#[test]
fn a_parentless_head_that_is_its_own_base_still_has_its_root_judged() {
    let repo = Repo::init(&[]);
    with_policy(&repo);
    let dev = key(&repo, "dev", "dev@example.com");
    repo.write(
        ".jtac/signers",
        signers_line(&dev, "dev@example.com").as_bytes(),
    );
    let path = dev.to_string_lossy().replace('\\', "/");
    repo.git(&["config", "jtac.signingkey", &path]);
    repo.git(&["config", "user.email", "dev@example.com"]);
    let only = repo.commit("only");
    ok(&repo, &["check"]);
    let (code, v) = json(
        &repo,
        &["ci", "--base", "HEAD", "--require-local-root", "--json"],
    );
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["waivable"], 1);
    assert_eq!(v["head_root"]["compared"], "agreed");
    let mut forged = read_root(&repo, &only);
    forged.counts.findings.minor += 1;
    put(&repo, &dev, &only, &forged);
    let (code, v) = ci(&repo);
    assert_eq!(code, 1, "{v}");
    assert_eq!(v["head_root"]["compared"], "disagreed");
}

#[test]
fn a_head_root_naming_another_policy_hash_lied_and_fails() {
    let p = pr(Trusts::Dev);
    let tip = p.add("a.txt", "a\n");
    ok(&p.repo, &["check"]);
    let mut forged = read_root(&p.repo, &tip);
    forged.policy.hash = "sha256:0000000000000000000000000000000000000000000000000000000000000000"
        .parse()
        .unwrap();
    put(&p.repo, &p.dev, &tip, &forged);
    let (code, v) = ci(&p.repo);
    assert_eq!(code, 1, "{v}");
    assert_eq!(v["verdict"]["pass"], true);
    assert_eq!(v["head_root"]["compared"], "lied");
    assert!(v["head_root"]["run"].is_null(), "{v}");
    let why = v["head_root"]["why"].as_str().unwrap();
    assert!(why.starts_with("it names policy sha256:0000"), "{why}");
    assert!(why.contains("this gate computes sha256:"), "{why}");
    three_parts(&v);
    assert!(
        v["reasons"][0]["what"]
            .as_str()
            .unwrap()
            .starts_with("HEAD's root lied about its policy"),
        "{v}"
    );
}

/// The developer judged `two` from `one` with `--base`, so the head root never saw the test
/// removal in `one`; CI diffs from the merge base and does.
#[test]
fn a_test_removed_before_the_diff_the_head_root_declares_fails_ci_and_the_root_still_agrees() {
    let p = pr(Trusts::Dev);
    let one = p.remove_test();
    let two = p.add("b.txt", "b\n");
    ok(&p.repo, &["check", "--base", &one]);
    let root = read_root(&p.repo, &two);
    assert_eq!(root.base, Some(g(&one)));
    let (code, v) = ci(&p.repo);
    assert_eq!(code, 1, "{v}");
    assert_eq!(rules(&v), ["JT-TEST-001"]);
    assert_eq!(v["verdict"]["pass"], false);
    assert_eq!(v["waivable"], 2);
    assert_eq!(v["head_root"]["compared"], "agreed");
    assert_eq!(v["reasons"], serde_json::json!([]));
    let ran = jtac(&p.repo, &["ci", "--base", "main"]);
    assert!(
        ran.stdout
            .starts_with("ABORT: 1 reasons block; head root agreed\n"),
        "{}",
        ran.stdout
    );
    assert!(
        ran.stdout
            .contains("  1. JT-TEST-001 blocker src/lib.rs: test `keeps` removed"),
        "{}",
        ran.stdout
    );
}

#[test]
fn a_trusted_commit_waiver_on_an_earlier_push_stays_waived_in_ci() {
    let p = pr(Trusts::Dev);
    let one = p.remove_test();
    ok(
        &p.repo,
        &["waive", "HEAD", "--reason", "the test was obsolete"],
    );
    ok(&p.repo, &["check"]);
    p.add("b.txt", "b\n");
    ok(&p.repo, &["check"]);
    let (code, v) = ci(&p.repo);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["verdict"]["state"], "declared_off");
    assert_eq!(rules(&v), ["JT-TEST-001"]);
    assert_eq!(
        v["verdict"]["waivers"][0]["scope"]["commit"],
        format!("sha1:{one}")
    );
    assert_eq!(v["withheld_waivers"], serde_json::json!([]));
    assert_eq!(v["head_root"]["compared"], "agreed");
    assert_eq!(v["pass"], true);
}

/// `one` removes a test and is waived by the trusted developer; `two`, pushed around the hook,
/// removes another. The waiver carries only across commits a trusted root covers.
#[test]
fn a_commit_waiver_carries_only_across_commits_a_trusted_root_covers() {
    let p = pr(Trusts::Dev);
    p.add("src/more.rs", "#[test]\nfn keeps2() {}\n");
    ok(&p.repo, &["check"]);
    let one = p.remove_test();
    ok(
        &p.repo,
        &["waive", "HEAD", "--reason", "the test was obsolete"],
    );
    ok(&p.repo, &["check"]);
    let two = p.add("src/more.rs", "fn b() {}\n");
    let (code, v) = ci(&p.repo);
    assert_eq!(code, 1, "{v}");
    assert_eq!(v["verdict"]["state"], "covered");
    assert_eq!(rules(&v), ["JT-TEST-001"]);
    let w = &v["withheld_waivers"][0];
    assert_eq!(w["commit"], format!("sha1:{one}"));
    assert_eq!(
        w["why"],
        format!(
            "1 commits after the one it names are not covered by a root base trusts, {} among \
             them",
            &two[..12]
        )
    );
    assert!(v["head_root"].is_null(), "{v}");
    assert_eq!(jtac(&p.repo, &["check"]).code, 1, "two is judged from one");
    ok(&p.repo, &["waive", "HEAD", "--reason", "that one too"]);
    ok(&p.repo, &["check"]);
    let (code, v) = ci(&p.repo);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["verdict"]["state"], "declared_off");
    assert_eq!(v["verdict"]["waivers"].as_array().unwrap().len(), 2);
    assert_eq!(v["withheld_waivers"], serde_json::json!([]));
    p.repo.git(&["switch", "-q", "main"]);
    p.add("m.txt", "m\n");
    ok(&p.repo, &["check"]);
    p.repo.git(&["switch", "-q", "feature"]);
    p.repo.git(&[
        "merge",
        "-q",
        "--no-ff",
        "--no-gpg-sign",
        "-m",
        "merge main",
        "main",
    ]);
    ok(&p.repo, &["check"]);
    let (code, v) = ci(&p.repo);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["verdict"]["state"], "declared_off");
    assert_eq!(
        v["withheld_waivers"],
        serde_json::json!([]),
        "base-branch commits merged in after the waiver are not the pull request's"
    );
    let other = p.other.to_string_lossy().replace('\\', "/");
    p.repo.git(&["config", "jtac.signingkey", &other]);
    p.repo.git(&["config", "user.email", "other@example.com"]);
    let late = p.add("late.txt", "late\n");
    ok(&p.repo, &["check"]);
    let (code, v) = ci(&p.repo);
    assert_eq!(code, 1, "{v}");
    assert_eq!(v["verdict"]["state"], "covered");
    assert_eq!(v["head_root"]["trust"], "unknown");
    let whys: Vec<&str> = v["withheld_waivers"]
        .as_array()
        .unwrap()
        .iter()
        .map(|w| w["why"].as_str().unwrap())
        .collect();
    assert_eq!(whys.len(), 2, "{v}");
    for why in whys {
        assert_eq!(
            why,
            format!(
                "1 commits after the one it names are not covered by a root base trusts, {} \
                 among them",
                &late[..12]
            ),
            "a root base does not trust covers nothing here"
        );
    }
}

/// The trusted developer's rule waiver, copied into a root signed by a key base does not
/// trust, clears nothing: the root is what put it in the chain.
#[test]
fn a_trusted_lever_carried_by_an_untrusted_root_is_withheld() {
    let p = pr(Trusts::Dev);
    let policy = std::fs::read_to_string(p.repo.path().join(".jtac/policy.toml")).unwrap();
    let tip = p.add(
        ".jtac/policy.toml",
        &format!("{policy}[integrity]\nmax_added_lines = 1000\n"),
    );
    ok(
        &p.repo,
        &[
            "waive",
            "--rule",
            "JT-PROT-001",
            "--path",
            ".jtac/",
            "--expires",
            "7d",
            "--reason",
            "reviewed",
        ],
    );
    ok(&p.repo, &["check"]);
    let (code, v) = ci(&p.repo);
    assert_eq!(code, 0, "{v}");
    let mut carried = read_root(&p.repo, &tip);
    carried.signer.principal = "other@example.com".into();
    carried.signer.fingerprint = fingerprint(&p.other);
    put(&p.repo, &p.other, &tip, &carried);
    let (code, v) = ci(&p.repo);
    assert_eq!(code, 1, "{v}");
    assert_eq!(rules(&v), ["JT-PROT-001"]);
    assert_eq!(v["verdict"]["waivers"], serde_json::json!([]));
    let w = &v["withheld_waivers"][0];
    assert_eq!(w["principal"], "dev@example.com");
    assert_eq!(
        w["why"],
        "the root carrying it is signed by a key not in signers"
    );
    assert_eq!(
        w["covers"],
        serde_json::json!([
            "JT-PROT-001 major .jtac/policy.toml: protected path `.jtac/policy.toml` changed"
        ])
    );
    assert_eq!(v["head_root"]["trust"], "unknown");
    assert_eq!(v["head_root"]["compared"], "agreed");
}

#[test]
fn a_finding_covered_only_by_an_untrusted_waiver_fails_naming_the_waiver() {
    let p = pr(Trusts::Dev);
    let one = p.remove_test();
    let other = p.other.to_string_lossy().replace('\\', "/");
    p.repo.git(&["config", "jtac.signingkey", &other]);
    p.repo.git(&["config", "user.email", "other@example.com"]);
    ok(
        &p.repo,
        &["waive", "HEAD", "--reason", "the test was obsolete"],
    );
    let dev = p.dev.to_string_lossy().replace('\\', "/");
    p.repo.git(&["config", "jtac.signingkey", &dev]);
    p.repo.git(&["config", "user.email", "dev@example.com"]);
    ok(&p.repo, &["check"]);
    p.add("b.txt", "b\n");
    ok(&p.repo, &["check"]);
    let (code, v) = ci(&p.repo);
    assert_eq!(code, 1, "{v}");
    assert_eq!(v["verdict"]["state"], "covered");
    assert_eq!(rules(&v), ["JT-TEST-001"]);
    assert_eq!(v["verdict"]["waivers"], serde_json::json!([]));
    let u = &v["withheld_waivers"][0];
    assert_eq!(u["commit"], format!("sha1:{one}"));
    assert_eq!(u["why"], "its signer is not in signers");
    assert_eq!(u["principal"], "other@example.com");
    assert_eq!(u["scope"]["commit"], format!("sha1:{one}"));
    assert!(
        u["covers"][0]
            .as_str()
            .unwrap()
            .starts_with("the verdict: a commit waiver in scope"),
        "{u}"
    );
    assert_eq!(v["head_root"]["trust"], "trusted");
    assert_eq!(v["head_root"]["compared"], "agreed");
    let ran = jtac(&p.repo, &["ci", "--base", "main"]);
    assert!(
        ran.stdout
            .contains("  waiver  not applied (its signer is not in signers): "),
        "{}",
        ran.stdout
    );
    assert!(
        ran.stdout.contains("          would cover the verdict"),
        "{}",
        ran.stdout
    );
}

#[test]
fn an_unknown_signer_is_reported_and_compared_and_a_rule_waiver_it_signed_names_what_it_covered() {
    let p = pr(Trusts::Other);
    p.remove_test();
    ok(
        &p.repo,
        &[
            "waive",
            "--rule",
            "JT-TEST-001",
            "--path",
            "src/",
            "--expires",
            "7d",
            "--reason",
            "obsolete",
        ],
    );
    ok(&p.repo, &["check"]);
    let (code, v) = ci(&p.repo);
    assert_eq!(code, 1, "{v}");
    assert_eq!(v["head_root"]["trust"], "unknown");
    assert_eq!(v["head_root"]["compared"], "agreed");
    assert_eq!(rules(&v), ["JT-TEST-001"]);
    assert_eq!(
        v["withheld_waivers"][0]["covers"],
        serde_json::json!(["JT-TEST-001 blocker src/lib.rs: test `keeps` removed"])
    );
    assert_eq!(v["reasons"], serde_json::json!([]));
}

#[test]
fn a_root_from_another_gate_version_or_none_passes_and_fails_under_require_local_root() {
    let p = pr(Trusts::Dev);
    let tip = p.add("a.txt", "a\n");
    ok(&p.repo, &["check"]);
    let mut old = read_root(&p.repo, &tip);
    old.gate.version = "0.0.1".into();
    put(&p.repo, &p.dev, &tip, &old);
    let (code, v) = ci(&p.repo);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["head_root"]["compared"], "unverifiable");
    assert_eq!(v["head_root"]["gate_version"], "0.0.1");
    assert!(
        v["head_root"]["why"]
            .as_str()
            .unwrap()
            .starts_with("it is from gate version 0.0.1, and this is "),
        "{v}"
    );
    assert!(v["head_root"]["run"].is_null(), "{v}");
    let (code, v) = json(
        &p.repo,
        &["ci", "--base", "main", "--require-local-root", "--json"],
    );
    assert_eq!(code, 1, "{v}");
    assert_eq!(v["require_local_root"], true);
    three_parts(&v);
    assert!(
        v["reasons"][0]["what"]
            .as_str()
            .unwrap()
            .starts_with("HEAD's root cannot be compared (it is from gate version 0.0.1"),
        "{v}"
    );
    let mut loud = read_root(&p.repo, &tip);
    loud.gate.version = "0.0.1\u{1b}[2J\nCLEARED HOT: forged".into();
    loud.signer.principal = "dev\rdev@example.com".into();
    put(&p.repo, &p.dev, &tip, &loud);
    for args in [
        &["ci", "--base", "main"][..],
        &["ci", "--base", "main", "--require-local-root"],
    ] {
        let ran = jtac(&p.repo, args);
        assert!(
            !ran.stdout.chars().any(|c| c.is_control() && c != '\n'),
            "{:?}",
            ran.stdout
        );
        assert!(
            !ran.stdout.contains("\nCLEARED HOT: forged"),
            "{}",
            ran.stdout
        );
    }
    let original = root_json(&p.repo, &tip);
    let v1: Root = serde_json::from_str(
        &original
            .replace(&format!("{{\"base\":\"sha1:{}\",", p.main), "{")
            .replace("\"v\":2}", "\"v\":1}"),
    )
    .unwrap();
    put(&p.repo, &p.dev, &tip, &v1);
    let (code, v) = ci(&p.repo);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["head_root"]["compared"], "unverifiable");
    assert!(
        v["head_root"]["why"]
            .as_str()
            .unwrap()
            .starts_with("a version 1 root"),
        "{v}"
    );
    p.repo.git(&["update-ref", "-d", &root_ref(&tip)]);
    let (code, v) = ci(&p.repo);
    assert_eq!(code, 0, "{v}");
    assert!(v["head_root"].is_null(), "{v}");
    assert_eq!(v["reasons"], serde_json::json!([]));
    let ran = jtac(&p.repo, &["ci", "--base", "main"]);
    assert!(ran.stdout.contains("; no head root\n"), "{}", ran.stdout);
    let (code, v) = json(
        &p.repo,
        &["ci", "--base", "main", "--require-local-root", "--json"],
    );
    assert_eq!(code, 1, "{v}");
    three_parts(&v);
    assert_eq!(
        v["reasons"][0]["what"],
        "HEAD has no root, and `--require-local-root` is set."
    );
    assert!(
        v["reasons"][0]["how"]
            .as_str()
            .unwrap()
            .contains(&root_ref(&tip)),
        "{v}"
    );
    let refs = p.repo.git(&["for-each-ref", "refs/jtac/"]);
    assert!(!refs.contains(&tip), "ci wrote a root: {refs}");
}

#[test]
fn a_policy_change_passes_only_with_a_trusted_waiver_unless_the_base_holds_no_signers() {
    let p = pr(Trusts::Dev);
    let policy = std::fs::read_to_string(p.repo.path().join(".jtac/policy.toml")).unwrap();
    p.add(
        ".jtac/policy.toml",
        &format!("{policy}[integrity]\nmax_added_lines = 1000\n"),
    );
    let (code, v) = ci(&p.repo);
    assert_eq!(code, 1, "{v}");
    assert_eq!(v["policy_from"], "base");
    assert_eq!(rules(&v), ["JT-PROT-001"]);
    assert!(v["head_root"].is_null(), "{v}");
    ok(
        &p.repo,
        &[
            "waive",
            "--rule",
            "JT-PROT-001",
            "--path",
            ".jtac/",
            "--expires",
            "7d",
            "--reason",
            "reviewed",
        ],
    );
    ok(&p.repo, &["check"]);
    let (code, v) = ci(&p.repo);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["policy_from"], "base");
    assert_eq!(v["verdict"]["waived"][0]["rule"], "JT-PROT-001");
    assert_eq!(v["verdict"]["waivers"][0]["scope"]["rule"], "JT-PROT-001");
    assert_eq!(v["head_root"]["compared"], "agreed", "{v}");
    assert_eq!(v["head_root"]["trust"], "trusted");

    let q = pr(Trusts::NoFile);
    q.repo.write(
        ".jtac/signers",
        signers_line(&q.dev, "dev@example.com").as_bytes(),
    );
    q.repo.write(".github/workflows/ci.yml", b"name: ci\n");
    q.add(
        ".jtac/policy.toml",
        &format!("{policy}[integrity]\nmax_added_lines = 1000\n"),
    );
    assert_eq!(
        jtac(&q.repo, &["check"]).code,
        1,
        "locally the change is major"
    );
    let (code, v) = ci(&q.repo);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["adoption"], true);
    assert!(
        v["signers"]
            .as_str()
            .unwrap()
            .starts_with("none (main has no .jtac/signers"),
        "{v}"
    );
    assert_eq!(v["policy_from"], "base");
    assert_eq!(rules(&v), Vec::<&str>::new());
    let advisory: Vec<&str> = v["verdict"]["advisory"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["path"].as_str().unwrap())
        .collect();
    assert_eq!(
        advisory,
        [
            ".github/workflows/ci.yml",
            ".jtac/policy.toml",
            ".jtac/signers"
        ]
    );
    assert!(
        v["verdict"]["advisory"][0]["message"]
            .as_str()
            .unwrap()
            .ends_with("the base holds no signers file, so nothing was in force to protect"),
        "{v}"
    );
    assert!(v["head_root"].is_null(), "{v}");
    let ran = jtac(&q.repo, &["ci", "--base", "main"]);
    assert!(
        ran.stdout
            .contains("(adoption: every protected-path finding is minor)"),
        "{}",
        ran.stdout
    );
    let file = q.repo.write(
        ".git/trust",
        signers_line(&q.dev, "dev@example.com").as_bytes(),
    );
    let (code, v) = json(
        &q.repo,
        &[
            "ci",
            "--base",
            "main",
            "--signers",
            file.to_str().unwrap(),
            "--json",
        ],
    );
    assert_eq!(code, 0, "{v}");
    assert_eq!(
        v["adoption"], true,
        "--signers supplies trust, not the adoption fact"
    );
    assert_eq!(v["signers"], file.to_str().unwrap());
}

#[test]
fn a_from_root_at_the_head_is_compared_in_its_scope_or_unverifiable_without_its_history() {
    let p = pr(Trusts::Dev);
    let tip = p.add("a.txt", "a\n");
    ok(&p.repo, &["check"]);
    let mut from = read_root(&p.repo, &tip);
    from.from = Some(FromHead {
        head: g(&p.main),
        root: None,
        compared: None,
    });
    put(&p.repo, &p.dev, &tip, &from);
    let (code, v) = ci(&p.repo);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["head_root"]["trust"], "unknown");
    assert_eq!(v["head_root"]["compared"], "agreed");
    from.from = Some(FromHead {
        head: g("abababababababababababababababababababab"),
        root: None,
        compared: None,
    });
    put(&p.repo, &p.dev, &tip, &from);
    let (code, v) = ci(&p.repo);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["head_root"]["compared"], "unverifiable");
    assert!(
        v["head_root"]["why"]
            .as_str()
            .unwrap()
            .starts_with("its scope needs history this clone lacks"),
        "{v}"
    );
    let (code, _) = json(
        &p.repo,
        &["ci", "--base", "main", "--require-local-root", "--json"],
    );
    assert_eq!(code, 1);
    p.repo.remove(".jtac/signers");
    p.repo.write(".jtac/signers/x", b"x\n");
    let broken = p.repo.commit("signers is a directory");
    std::fs::remove_dir_all(p.repo.path().join(".jtac/signers")).unwrap();
    p.repo.write(
        ".jtac/signers",
        signers_line(&p.dev, "dev@example.com").as_bytes(),
    );
    let restored = p.repo.commit("signers restored");
    ok(&p.repo, &["check"]);
    let mut from = read_root(&p.repo, &restored);
    from.base = Some(g(&broken));
    from.covers = Covers::List(vec![g(&restored)]);
    from.parents = vec![];
    from.from = Some(FromHead {
        head: g(&broken),
        root: None,
        compared: None,
    });
    put(&p.repo, &p.dev, &restored, &from);
    let (code, v) = ci(&p.repo);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["head_root"]["compared"], "unverifiable");
    assert!(
        v["head_root"]["why"]
            .as_str()
            .unwrap()
            .starts_with("its base's tree could not be read: "),
        "{v}"
    );
}

/// The head edits the policy with a trusted rule waiver, so its local root is judged under
/// HEAD's policy; recast as a `from` root it should have been judged under its base's, the hash
/// `main`'s own root recorded. Then `main` moves to a third policy: the comparison must still
/// read the root's declared base, not the base branch the verifier was given.
#[test]
fn a_from_root_is_compared_under_its_declared_base_policy_after_the_base_branch_moves() {
    let p = pr(Trusts::Dev);
    let p0 = read_root(&p.repo, &p.main).policy.hash;
    let policy = std::fs::read_to_string(p.repo.path().join(".jtac/policy.toml")).unwrap();
    let tip = p.add(
        ".jtac/policy.toml",
        &format!("{policy}[integrity]\nmax_added_lines = 1000\n"),
    );
    ok(
        &p.repo,
        &[
            "waive",
            "--rule",
            "JT-PROT-001",
            "--path",
            ".jtac/",
            "--expires",
            "7d",
            "--reason",
            "reviewed",
        ],
    );
    ok(&p.repo, &["check"]);
    let mut from = read_root(&p.repo, &tip);
    assert_ne!(
        from.policy.hash, p0,
        "the local root is under HEAD's policy"
    );
    from.policy.hash = p0;
    from.from = Some(FromHead {
        head: g(&p.main),
        root: None,
        compared: None,
    });
    put(&p.repo, &p.dev, &tip, &from);
    p.repo.git(&["switch", "-q", "main"]);
    p.repo.write(
        ".jtac/policy.toml",
        format!("{policy}[integrity]\nmax_added_lines = 2000\n").as_bytes(),
    );
    p.repo.commit("main moves");
    p.repo.git(&["switch", "-q", "feature"]);
    let (code, v) = ci(&p.repo);
    assert_eq!(code, 1, "{v}");
    assert_eq!(v["policy_from"], "base");
    assert_eq!(v["head_root"]["compared"], "agreed", "{v}");
    assert_eq!(v["head_root"]["trust"], "unknown");
    assert_eq!(
        v["withheld_waivers"][0]["why"], "the root carrying it is signed by a key not in signers",
        "a from root reads as an unknown signer until M16, so its lever is withheld"
    );
}

#[test]
fn ci_refuses_a_base_that_does_not_resolve_a_dirty_tree_and_a_missing_policy_with_exit_two() {
    let p = pr(Trusts::Dev);
    p.add("a.txt", "a\n");
    let ran = jtac(&p.repo, &["ci", "--base", "nope"]);
    assert_eq!((ran.code, ran.stdout.as_str()), (2, ""), "{}", ran.stderr);
    assert!(
        ran.stderr
            .starts_with("jtac: `nope` does not name a commit."),
        "{}",
        ran.stderr
    );
    assert!(
        ran.stderr.contains("negotiation: Run `git rev-parse nope`"),
        "{}",
        ran.stderr
    );
    p.repo.write("dirty.txt", b"x\n");
    let ran = jtac(&p.repo, &["ci", "--base", "main"]);
    assert_eq!(ran.code, 2, "{}", ran.stdout);
    assert!(
        ran.stderr
            .starts_with("jtac: the work tree is not HEAD's tree"),
        "{}",
        ran.stderr
    );
    p.repo.remove("dirty.txt");
    p.repo.write(".jtac/policy.toml", b"version = 99\n");
    p.repo.commit("unreadable policy");
    let (code, v) = ci(&p.repo);
    assert_eq!(code, 1, "{v}");
    assert_eq!(v["policy_from"], "base");
    assert_eq!(rules(&v), ["JT-PROT-001"]);
    assert!(
        v["verdict"]["notes"][0]
            .as_str()
            .unwrap()
            .starts_with("HEAD's policy does not read ("),
        "{v}"
    );
    p.repo.remove(".jtac/policy.toml");
    p.repo.commit("no policy");
    let (code, v) = ci(&p.repo);
    assert_eq!(code, 1, "{v}");
    assert_eq!(v["policy_from"], "base");
    assert_eq!(rules(&v), ["JT-PROT-001"]);
    let early = Repo::init(&[]);
    early.write("a.txt", b"a\n");
    early.commit("before adoption");
    early.git(&["switch", "-q", "-c", "feature"]);
    early.write("b.txt", b"b\n");
    early.commit("feature");
    early.git(&["switch", "-q", "main"]);
    with_policy(&early);
    early.commit("adopts");
    early.git(&["switch", "-q", "feature"]);
    let (code, v) = ci(&early);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["policy_from"], "base");
    assert_eq!(v["verdict"]["notes"], serde_json::json!([]));
    let bare = Repo::init(&[]);
    bare.write("a.txt", b"a\n");
    bare.commit("main");
    bare.git(&["switch", "-q", "-c", "feature"]);
    bare.write("b.txt", b"b\n");
    bare.commit("feature");
    let ran = jtac(&bare, &["ci", "--base", "main"]);
    assert_eq!(ran.code, 2, "{}", ran.stdout);
    assert!(
        ran.stderr
            .starts_with("jtac: HEAD has no `.jtac/policy.toml`."),
        "{}",
        ran.stderr
    );
}
