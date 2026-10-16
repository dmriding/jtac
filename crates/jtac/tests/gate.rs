//! `jtac check`, `status`, and `why` against throwaway repositories with a throwaway ed25519
//! key (DESIGN.md sections 11.1 and 11.9): what a pass writes, what a block writes, what a
//! second check keeps, and every refusal, through the built binary.

use std::process::Command;

use jtac_testkit::Repo;
use serde_json::Value;

fn me() -> String {
    env!("CARGO_BIN_EXE_jtac").replace('\\', "/")
}

fn policy(extra: &str) -> String {
    format!(
        "version = 1\n[checks.ok]\nrun = [\"{}\", \"--version\"]\n{extra}",
        me()
    )
}

/// The binary under coverage drops a profile file where it runs; ignored, it leaves the tree
/// clean for the next check.
fn with_policy(repo: &Repo, extra: &str) {
    repo.write(".jtac/policy.toml", policy(extra).as_bytes());
    repo.write(".gitignore", b"*.profraw\n");
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

fn json(repo: &Repo, args: &[&str]) -> (i32, Value) {
    let ran = jtac(repo, args);
    assert!(ran.stderr.is_empty(), "{}", ran.stderr);
    (ran.code, serde_json::from_str(&ran.stdout).unwrap())
}

/// A key under `.git/`, where the work tree stays clean, named in the local config with the
/// principal.
fn keyed(repo: &Repo) {
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
}

/// A clean repository with a passing policy and a key.
fn passing() -> Repo {
    let repo = Repo::init(&[]);
    with_policy(&repo, "");
    repo.write("src/lib.rs", b"fn a() {}\n");
    repo.commit("base");
    keyed(&repo);
    repo
}

/// `main` holds a test; `feature` removes it, so the rules block.
fn blocking() -> Repo {
    let repo = Repo::init(&[]);
    with_policy(&repo, "");
    repo.write("src/lib.rs", b"#[test]\nfn keeps() {}\nfn a() {}\n");
    repo.commit("base");
    repo.git(&["switch", "-q", "-c", "feature"]);
    repo.write("src/lib.rs", b"fn a() {}\n");
    repo.commit("tip");
    keyed(&repo);
    repo
}

fn root_refs(repo: &Repo) -> Vec<String> {
    repo.git(&["for-each-ref", "--format=%(refname)", "refs/jtac/roots/"])
        .lines()
        .map(str::to_owned)
        .collect()
}

fn root_json(repo: &Repo, commit: &str) -> Value {
    let name = format!("refs/jtac/roots/{}/{commit}", &commit[..2]);
    let text = repo.git(&["cat-file", "blob", &format!("{name}:root.json")]);
    serde_json::from_str(&text).unwrap()
}

fn state(repo: &Repo) -> Value {
    let dir = repo.git(&["rev-parse", "--git-common-dir"]);
    let bytes = std::fs::read(repo.path().join(dir).join("jtac").join("state.json")).unwrap();
    serde_json::from_slice(&bytes).unwrap()
}

#[test]
fn a_pass_writes_a_signed_root_that_ssh_keygen_accepts_and_records_the_state() {
    let repo = passing();
    let head = repo.git(&["rev-parse", "HEAD"]);
    let ran = jtac(&repo, &["check"]);
    assert_eq!(ran.code, 0, "{}", ran.stderr);
    assert!(ran.stdout.starts_with("CLEARED HOT: "), "{}", ran.stdout);
    assert!(ran.stdout.contains("  root    sha256:"), "{}", ran.stdout);
    assert!(!ran.stdout.contains("preview"), "{}", ran.stdout);
    let refs = root_refs(&repo);
    assert_eq!(refs.len(), 1, "{refs:?}");
    assert!(refs[0].ends_with(&head), "{refs:?}");
    let root = root_json(&repo, &head);
    assert_eq!(root["commit"], format!("sha1:{head}"));
    assert_eq!(root["covers"], serde_json::json!([format!("sha1:{head}")]));
    assert_eq!(root["parents"], serde_json::json!([]));
    assert_eq!(root["state"], "covered");
    assert_eq!(root["signer"]["principal"], "dev@example.com");
    assert_eq!(root["gate"]["version"], env!("CARGO_PKG_VERSION"));
    assert_eq!(root["counts"]["checks"], 2);
    assert_eq!(root["counts"]["pass"], 2);
    assert_eq!(root["policy"]["source"], "repo");
    let name = format!("refs/jtac/roots/{}/{head}", &head[..2]);
    let sig = repo.git(&["cat-file", "blob", &format!("{name}:root.sig")]);
    assert!(sig.starts_with("-----BEGIN SSH SIGNATURE-----"), "{sig}");
    let sig_path = repo.write("check.sig", format!("{sig}\n").as_bytes());
    let bytes = repo.command("git", &["cat-file", "blob", &format!("{name}:root.json")]);
    let allowed = repo.write(
        "allowed",
        format!(
            "dev@example.com {}\n",
            std::fs::read_to_string(repo.path().join(".git").join("key.pub")).unwrap()
        )
        .as_bytes(),
    );
    let mut verify = Command::new("ssh-keygen")
        .args([
            "-Y",
            "verify",
            "-n",
            "jtac-root-v1",
            "-I",
            "dev@example.com",
            "-f",
        ])
        .arg(&allowed)
        .arg("-s")
        .arg(&sig_path)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    std::io::Write::write_all(verify.stdin.as_mut().unwrap(), &bytes.stdout).unwrap();
    let out = verify.wait_with_output().unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let s = state(&repo);
    assert_eq!(s["v"], 1);
    assert_eq!(s["pass"], true);
    assert_eq!(s["preview"], false);
    assert_eq!(s["commit"], format!("sha1:{head}"));
    assert!(s["root"].as_str().unwrap().starts_with("sha256:"), "{s}");
    let (code, v) = json(&repo, &["why", "--json"]);
    assert_eq!(code, 0);
    assert_eq!(v["root"], s["root"]);
    assert_eq!(v["notes"], serde_json::json!([]));
    let ran = jtac(&repo, &["status"]);
    assert_eq!(ran.code, 0, "{}", ran.stderr);
    assert!(
        ran.stdout.starts_with("PUSH WOULD PASS: HEAD"),
        "{}",
        ran.stdout
    );
    assert!(ran.stdout.contains("upstream none"), "{}", ran.stdout);
    assert!(
        ran.stdout.contains("last    CLEARED HOT at"),
        "{}",
        ran.stdout
    );
}

#[test]
fn a_block_writes_nodes_and_the_state_but_no_root_and_why_explains_it() {
    let repo = blocking();
    let ran = jtac(&repo, &["check"]);
    assert_eq!(ran.code, 1, "{}", ran.stderr);
    assert!(
        ran.stdout.starts_with("ABORT: 1 reasons block"),
        "{}",
        ran.stdout
    );
    assert!(
        ran.stdout.contains("  root    none written"),
        "{}",
        ran.stdout
    );
    assert!(root_refs(&repo).is_empty());
    let dir = repo.git(&["rev-parse", "--git-common-dir"]);
    let objects = std::fs::read_dir(repo.path().join(dir).join("jtac").join("objects")).unwrap();
    assert!(objects.count() >= 3, "integrity, finding, and ok nodes");
    let s = state(&repo);
    assert_eq!(
        (s["pass"].as_bool(), s["root"].is_null()),
        (Some(false), true)
    );
    let ran = jtac(&repo, &["why"]);
    assert_eq!(ran.code, 1, "{}", ran.stderr);
    assert!(
        ran.stdout.starts_with("ABORT: 1 reasons block"),
        "{}",
        ran.stdout
    );
    assert!(
        ran.stdout.contains("1. JT-TEST-001 blocker"),
        "{}",
        ran.stdout
    );
    assert!(
        ran.stdout.contains("fix  restore the test"),
        "{}",
        ran.stdout
    );
    let ran = jtac(&repo, &["status"]);
    assert_eq!(ran.code, 1);
    assert!(
        ran.stdout.starts_with("PUSH WOULD BE REFUSED: HEAD"),
        "{}",
        ran.stdout
    );
    assert!(
        ran.stdout.contains("fix     run `jtac check`"),
        "{}",
        ran.stdout
    );
    assert!(ran.stdout.contains("last    ABORT at"), "{}", ran.stdout);
    repo.write("src/lib.rs", b"fn a() {}\nfn b() {}\n");
    repo.commit("moved");
    let (code, v) = json(&repo, &["why", "--json"]);
    assert_eq!(code, 1);
    let note = v["notes"][0].as_str().unwrap();
    assert!(note.starts_with("HEAD is now "), "{note}");
    assert!(note.ends_with("run `jtac check`"), "{note}");
}

#[test]
fn a_second_check_on_a_covered_head_keeps_the_root_and_the_next_commits_chain_to_it() {
    let repo = passing();
    let first = repo.git(&["rev-parse", "HEAD"]);
    let (code, v) = json(&repo, &["check", "--json"]);
    assert_eq!(code, 0);
    let root_id = v["root"].as_str().unwrap().to_owned();
    let (code, v) = json(&repo, &["check", "--json"]);
    assert_eq!(code, 0);
    assert_eq!(v["root"], root_id);
    assert_eq!(
        v["base"]["from"],
        "HEAD itself (HEAD has no parent, so there is no change to judge)"
    );
    let notes: Vec<&str> = v["notes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|n| n.as_str().unwrap())
        .collect();
    assert_eq!(
        notes,
        [
            "HEAD already has a root; it stands, and this run wrote none: the state and waivers \
             shown are the root's"
        ]
    );
    assert_eq!(root_refs(&repo).len(), 1);
    repo.write("src/lib.rs", b"fn a() {}\nfn b() {}\n");
    let second = repo.commit("two");
    repo.write("src/lib.rs", b"fn a() {}\nfn b() {}\nfn c() {}\n");
    let third = repo.commit("three");
    let (code, v) = json(&repo, &["check", "--json"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["base"]["commit"], first);
    assert_eq!(v["base"]["from"], "nearest covered ancestor");
    let root = root_json(&repo, &third);
    assert_eq!(
        root["covers"],
        serde_json::json!([format!("sha1:{second}"), format!("sha1:{third}")])
    );
    assert_eq!(
        root["parents"],
        serde_json::json!([{ "commit": format!("sha1:{first}"), "root": root_id }])
    );
    assert_eq!(root_refs(&repo).len(), 2);
}

/// The probe table knows `node`, and this binary answering `--version` under that name gives
/// a tool version without a toolchain the scrubbed check environment cannot find.
/// Issue 27: the check in the policy fetches the other clone's root refs, so the tip's ref
/// appears between this run's look at it and its own write. The ref is created, never
/// replaced; the loser reports the root that won.
#[test]
fn a_check_that_loses_the_race_to_the_same_tip_reports_the_root_that_won() {
    let first = Repo::init(&[]);
    let path = first.path().to_string_lossy().replace('\\', "/");
    with_policy(
        &first,
        &format!(
            "[checks.fetch]\nrun = [\"git\", \"fetch\", \"-q\", \"{path}\", \
             \"+refs/jtac/*:refs/jtac/*\"]\n"
        ),
    );
    first.write("src/lib.rs", b"fn a() {}\n");
    let head = first.commit("base");
    keyed(&first);
    let (code, out) = json(&first, &["check", "--json"]);
    assert_eq!(code, 0, "{out}");
    let won = out["root"].as_str().unwrap().to_owned();

    let second = Repo::clone_of(&first);
    keyed(&second);
    assert_eq!(second.git(&["rev-parse", "HEAD"]), head);
    assert!(root_refs(&second).is_empty());
    let (code, out) = json(&second, &["check", "--json"]);
    assert_eq!(code, 0, "{out}");
    assert_eq!(out["root"].as_str().unwrap(), won);
    assert!(
        out["notes"].as_array().unwrap().iter().any(|n| n
            .as_str()
            .unwrap()
            .contains("another run wrote HEAD's root first")),
        "{out}"
    );
    assert_eq!(root_refs(&second).len(), 1);
    assert_eq!(
        root_json(&second, &head)["ts"],
        root_json(&first, &head)["ts"]
    );
}

#[test]
fn a_known_tool_is_a_cache_hit_on_the_second_check() {
    let repo = Repo::init(&[]);
    let exe = std::path::Path::new(env!("CARGO_BIN_EXE_jtac"));
    let mut node = repo.path().join(".git").join("node");
    if let Some(ext) = exe.extension() {
        node.set_extension(ext);
    }
    std::fs::copy(exe, &node).unwrap();
    let extra = format!(
        "[checks.ver]\nrun = [\"{}\", \"--version\"]\n",
        node.to_string_lossy().replace('\\', "/")
    );
    with_policy(&repo, &extra);
    repo.commit("base");
    keyed(&repo);
    let (code, v) = json(&repo, &["check", "--preview", "--json"]);
    assert_eq!(code, 0, "{v}");
    let (code, v) = json(&repo, &["check", "--json"]);
    assert_eq!(code, 0, "{v}");
    let ver = |v: &Value| {
        v["checks"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["id"] == "ver")
            .unwrap()
            .clone()
    };
    assert!(
        ver(&v)["cached_from"].is_null(),
        "preview wrote nothing: {v}"
    );
    let (_, v) = json(&repo, &["check", "--json"]);
    assert!(
        ver(&v)["cached_from"]
            .as_str()
            .unwrap()
            .starts_with("sha256:"),
        "{v}"
    );
    let ran = jtac(&repo, &["check"]);
    assert!(
        ran.stdout.contains("ver pass 0ms (cached)"),
        "{}",
        ran.stdout
    );
    let (_, v) = json(&repo, &["check", "--preview", "--json"]);
    assert!(
        ver(&v)["cached_from"].is_null(),
        "preview never reads the cache: {v}"
    );
}

#[test]
fn a_tree_that_status_calls_clean_can_still_differ_from_head() {
    let repo = passing();
    let refused = |want: &str| {
        let ran = jtac(&repo, &["check"]);
        assert_eq!(ran.code, 2, "{}{}", ran.stdout, ran.stderr);
        assert!(
            ran.stderr
                .starts_with("jtac: the work tree is not HEAD's tree: "),
            "{}",
            ran.stderr
        );
        assert!(ran.stderr.contains(want), "{want}: {}", ran.stderr);
        assert!(root_refs(&repo).is_empty());
    };
    repo.write("src/lib.rs", b"fn a() {}\nfn b() {}\n");
    refused("`git status` shows uncommitted changes");
    repo.git(&["update-index", "--skip-worktree", "src/lib.rs"]);
    assert_eq!(repo.git(&["status", "--porcelain"]), "");
    refused("`src/lib.rs` is marked skip-worktree or assume-unchanged");
    repo.git(&["update-index", "--no-skip-worktree", "src/lib.rs"]);
    repo.git(&["update-index", "--assume-unchanged", "src/lib.rs"]);
    refused("`src/lib.rs` is marked skip-worktree or assume-unchanged");
    repo.git(&["update-index", "--no-assume-unchanged", "src/lib.rs"]);
    repo.git(&["checkout", "--", "src/lib.rs"]);
    repo.write("helper/mod.py", b"shadow\n");
    repo.write(".git/info/exclude", b"helper/\n");
    assert_eq!(repo.git(&["status", "--porcelain"]), "");
    refused("`helper/mod.py` is untracked and hidden by a local exclude");
    std::fs::remove_dir_all(repo.path().join("helper")).unwrap();
    assert_eq!(jtac(&repo, &["check"]).code, 0, "clean again");
}

#[test]
fn a_dirty_tree_is_refused_and_preview_still_answers() {
    let repo = passing();
    repo.write("extra.txt", b"untracked\n");
    let ran = jtac(&repo, &["check"]);
    assert_eq!(ran.code, 2);
    assert!(ran.stdout.is_empty());
    assert!(
        ran.stderr
            .starts_with("jtac: the work tree is not HEAD's tree: "),
        "{}",
        ran.stderr
    );
    assert!(
        ran.stderr.contains("negotiation: Commit or stash"),
        "{}",
        ran.stderr
    );
    assert!(root_refs(&repo).is_empty());
    let ran = jtac(&repo, &["check", "--preview"]);
    assert_eq!(ran.code, 0, "{}", ran.stderr);
    assert!(
        ran.stdout
            .starts_with("CLEARED HOT (preview, nothing written)"),
        "{}",
        ran.stdout
    );
    assert!(!ran.stdout.contains("  root"), "{}", ran.stdout);
    assert!(root_refs(&repo).is_empty());
    let ran = jtac(&repo, &["why"]);
    assert_eq!(ran.code, 2);
    assert!(
        ran.stderr.starts_with("jtac: no verdict has been recorded"),
        "{}",
        ran.stderr
    );
}

#[test]
fn no_key_and_no_principal_are_refused_before_any_check_runs() {
    let repo = Repo::init(&[]);
    with_policy(&repo, "");
    repo.commit("base");
    let ran = jtac(&repo, &["check"]);
    assert_eq!(ran.code, 2);
    assert!(
        ran.stderr.starts_with("jtac: cannot sign: no signing key"),
        "{}",
        ran.stderr
    );
    assert!(ran.stderr.contains("Run `jtac init`"), "{}", ran.stderr);
    repo.git(&["config", "gpg.format", "ssh"]);
    repo.git(&["config", "user.signingkey", "/nowhere/key"]);
    let ran = jtac(&repo, &["check"]);
    assert!(
        ran.stderr.starts_with("jtac: cannot sign: no principal"),
        "{}",
        ran.stderr
    );
    assert!(
        ran.stderr.contains("git config user.email"),
        "{}",
        ran.stderr
    );
    repo.git(&["config", "user.email", "dev@example.com"]);
    let ran = jtac(&repo, &["check", "--json"]);
    assert_eq!(ran.code, 2);
    assert!(
        ran.stderr.contains("\"what\": \"cannot sign: /nowhere/key"),
        "{}",
        ran.stderr
    );
    let dir = repo.git(&["rev-parse", "--git-common-dir"]);
    assert!(
        !repo.path().join(dir).join("jtac").exists(),
        "nothing ran, nothing written"
    );
}

#[test]
fn status_counts_the_unpushed_commits_by_their_own_roots() {
    let origin = Repo::bare(&[]);
    let repo = Repo::clone_of(&origin);
    with_policy(&repo, "");
    repo.commit("base");
    repo.git(&["push", "-q", "-u", "origin", "main"]);
    keyed(&repo);
    repo.write("a.txt", b"a\n");
    let first = repo.commit("one");
    assert_eq!(jtac(&repo, &["check"]).code, 0);
    repo.write("b.txt", b"b\n");
    let second = repo.commit("two");
    let (code, v) = json(&repo, &["status", "--json"]);
    assert_eq!(code, 1);
    assert_eq!(v["v"], 1);
    assert_eq!(v["command"], "status");
    assert_eq!(v["pass"], false);
    assert_eq!(v["upstream"], "origin/main");
    assert_eq!(
        v["unpushed"],
        serde_json::json!([
            { "commit": format!("sha1:{first}"), "covered": true },
            { "commit": format!("sha1:{second}"), "covered": false }
        ])
    );
    assert_eq!(v["last"]["commit"], format!("sha1:{first}"));
    let ran = jtac(&repo, &["status"]);
    assert!(
        ran.stdout
            .contains("upstream origin/main: 2 unpushed, 1 covered, 1 absent"),
        "{}",
        ran.stdout
    );
    assert!(
        ran.stdout.contains(&format!("{second} absent")),
        "{}",
        ran.stdout
    );
    assert_eq!(jtac(&repo, &["check"]).code, 0);
    let (code, v) = json(&repo, &["status", "--json"]);
    assert_eq!(code, 0);
    assert_eq!(v["unpushed"][1]["covered"], true);
    repo.git(&["push", "-q", "origin", "main"]);
    let ran = jtac(&repo, &["status"]);
    assert_eq!(ran.code, 0);
    assert!(
        ran.stdout.contains("upstream origin/main: nothing to push"),
        "{}",
        ran.stdout
    );
}

#[test]
fn a_root_for_a_descendant_or_a_missing_commit_does_not_block_head() {
    let repo = passing();
    let main = repo.git(&["rev-parse", "HEAD"]);
    repo.git(&["switch", "-q", "-c", "feat"]);
    repo.write("f.txt", b"f\n");
    let feat = repo.commit("feat");
    assert_eq!(jtac(&repo, &["check"]).code, 0);
    repo.git(&["switch", "-q", "main"]);
    repo.git(&[
        "update-ref",
        "refs/jtac/roots/00/0000000000000000000000000000000000000001",
        &feat,
    ]);
    let (code, v) = json(&repo, &["check", "--json"]);
    assert_eq!(code, 0, "{v}");
    let root = root_json(&repo, &main);
    assert_eq!(root["covers"], serde_json::json!([format!("sha1:{main}")]));
    assert_eq!(root["parents"], serde_json::json!([]));
    let (code, v) = json(&repo, &["status", "--json"]);
    assert_eq!(code, 0, "{v}");
}

#[test]
fn the_nearest_covered_ancestor_is_on_the_first_parent_line_or_none_at_all() {
    let repo = passing();
    let base = repo.git(&["rev-parse", "HEAD"]);
    repo.git(&["switch", "-q", "-c", "other"]);
    repo.write("s1.txt", b"s1\n");
    let s1 = repo.commit("s1");
    repo.write("x.txt", b"x\n");
    let x = repo.commit("x");
    assert_eq!(jtac(&repo, &["check"]).code, 0);
    repo.git(&["switch", "-q", "main"]);
    repo.write("t1.txt", b"t1\n");
    let t1 = repo.commit("t1");
    repo.git(&[
        "merge",
        "-q",
        "--no-ff",
        "--no-gpg-sign",
        "-m",
        "merge",
        "other",
    ]);
    let merge = repo.git(&["rev-parse", "HEAD"]);
    let (code, v) = json(&repo, &["check", "--json"]);
    assert_eq!(code, 0, "{v}");
    let from = v["base"]["from"].as_str().unwrap();
    let diffed = v["base"]["commit"].as_str().unwrap();
    assert!(diffed != s1 && diffed != base && diffed != x, "{v}");
    assert_eq!(diffed, t1, "{v}");
    assert!(from.starts_with("first parent of HEAD"), "{from}");
    let root = root_json(&repo, &merge);
    assert_eq!(root["covers"], serde_json::json!([format!("sha1:{merge}")]));
    assert_eq!(root["parents"][0]["commit"], format!("sha1:{x}"));
    assert_eq!(root["parents"].as_array().unwrap().len(), 1, "{root}");
}

#[test]
fn past_the_bound_covers_is_a_range_and_a_lost_state_file_only_warns() {
    let repo = passing();
    assert_eq!(jtac(&repo, &["check"]).code, 0);
    let mut first = String::new();
    for i in 0..257 {
        repo.git(&["commit", "-q", "--allow-empty", "--no-gpg-sign", "-m", "n"]);
        if i == 0 {
            first = repo.git(&["rev-parse", "HEAD"]);
        }
    }
    let tip = repo.git(&["rev-parse", "HEAD"]);
    let dir = repo.git(&["rev-parse", "--git-common-dir"]);
    let state_path = repo.path().join(dir).join("jtac").join("state.json");
    std::fs::remove_file(&state_path).unwrap();
    std::fs::create_dir(&state_path).unwrap();
    let ran = jtac(&repo, &["check"]);
    assert_eq!(ran.code, 0, "{}", ran.stderr);
    assert!(ran.stdout.starts_with("CLEARED HOT: "), "{}", ran.stdout);
    assert!(
        ran.stderr
            .starts_with("jtac: the local store could not answer: "),
        "{}",
        ran.stderr
    );
    let root = root_json(&repo, &tip);
    assert_eq!(
        root["covers"],
        serde_json::json!({ "base": format!("sha1:{first}"), "tip": format!("sha1:{tip}") })
    );
    let ran = jtac(&repo, &["status"]);
    assert_eq!(ran.code, 0);
    assert!(
        ran.stdout
            .contains("  note    the local store could not answer"),
        "{}",
        ran.stdout
    );
    assert!(
        ran.stdout.contains("last    no check has run"),
        "{}",
        ran.stdout
    );
}

/// Run `jtac` from `subdir` of the repository.
fn jtac_in(repo: &Repo, subdir: &str, args: &[&str]) -> Ran {
    let out = Command::new(env!("CARGO_BIN_EXE_jtac"))
        .args(args)
        .current_dir(repo.path().join(subdir))
        .env_clear()
        .envs(repo.env())
        .output()
        .unwrap();
    Ran {
        code: out.status.code().unwrap_or(-1),
        stdout: String::from_utf8(out.stdout).unwrap(),
        stderr: String::from_utf8(out.stderr).unwrap(),
    }
}

#[test]
fn a_tree_that_changes_under_the_checks_gets_no_root() {
    let repo = passing();
    repo.git(&["branch", "other"]);
    let rows = [
        (
            "[checks.mutate]\nrun = [\"git\", \"switch\", \"-q\", \"other\"]\n",
            "jtac: HEAD changed while the checks ran.",
        ),
        (
            "[checks.mutate]\nrun = [\"git\", \"init\", \"-q\", \"scratch\"]\n",
            "jtac: the work tree changed while the checks ran: `git status` shows",
        ),
    ];
    for (mutate, want) in rows {
        repo.git(&["switch", "-q", "--discard-changes", "main"]);
        repo.write(".jtac/policy.toml", policy(mutate).as_bytes());
        repo.commit("mutating policy");
        let ran = jtac(&repo, &["check"]);
        assert_eq!(ran.code, 2, "{}{}", ran.stdout, ran.stderr);
        assert!(ran.stderr.starts_with(want), "{}", ran.stderr);
        assert!(root_refs(&repo).is_empty());
        let dir = repo.git(&["rev-parse", "--git-common-dir"]);
        let state = repo.path().join(dir).join("jtac").join("state.json");
        assert!(!state.exists());
        let _ = std::fs::remove_dir_all(repo.path().join("scratch"));
    }
}

#[test]
fn a_shallow_mark_real_or_forged_gets_no_root() {
    let repo = Repo::init(&[]);
    with_policy(&repo, "");
    repo.write("src/lib.rs", b"#[test]\nfn keeps() {}\nfn a() {}\n");
    repo.commit("base");
    keyed(&repo);
    assert_eq!(jtac(&repo, &["check"]).code, 0);
    repo.write("src/lib.rs", b"fn a() {}\n");
    let dropped = repo.commit("drop test");
    repo.write("b.txt", b"b\n");
    let harmless = repo.commit("harmless");
    repo.write("c.txt", b"c\n");
    repo.commit("harmless again");
    repo.write(".git/shallow", format!("{harmless}\n").as_bytes());
    let refused = || {
        let ran = jtac(&repo, &["check"]);
        assert_eq!(ran.code, 2, "{}{}", ran.stdout, ran.stderr);
        assert!(
            ran.stderr.starts_with("jtac: the history is shallow: "),
            "{}",
            ran.stderr
        );
        assert!(ran.stderr.contains("--unshallow"), "{}", ran.stderr);
        assert_eq!(root_refs(&repo).len(), 1);
    };
    refused();
    let object = repo
        .path()
        .join(".git/objects")
        .join(&dropped[..2])
        .join(&dropped[2..]);
    std::fs::remove_file(object).unwrap();
    refused();
    let ran = jtac(&repo, &["check", "--preview"]);
    assert_eq!(ran.code, 0, "{}", ran.stderr);
    assert!(
        ran.stdout.contains("first parent of HEAD"),
        "{}",
        ran.stdout
    );
}

#[test]
fn a_merge_is_judged_against_its_first_parent_line_not_the_merged_branch() {
    let repo = Repo::init(&[]);
    with_policy(&repo, "");
    repo.write("src/lib.rs", b"fn a() {}\n");
    repo.commit("base");
    keyed(&repo);
    assert_eq!(jtac(&repo, &["check"]).code, 0);
    repo.git(&["switch", "-q", "-c", "side"]);
    repo.write("side.txt", b"s\n");
    repo.commit("side");
    assert_eq!(jtac(&repo, &["check"]).code, 0);
    repo.git(&["switch", "-q", "main"]);
    repo.write("src/lib.rs", b"#[test]\nfn keeps() {}\nfn a() {}\n");
    let added = repo.commit("add test");
    assert_eq!(jtac(&repo, &["check"]).code, 0);
    repo.write("src/lib.rs", b"fn a() {}\n");
    repo.commit("drop test");
    repo.git(&[
        "merge",
        "-q",
        "--no-ff",
        "--no-gpg-sign",
        "-m",
        "merge side",
        "side",
    ]);
    let (code, v) = json(&repo, &["check", "--json"]);
    assert_eq!(code, 1, "{v}");
    assert_eq!(v["base"]["commit"], added);
    assert_eq!(v["base"]["from"], "nearest covered ancestor");
    assert_eq!(v["reasons"][0]["rule"], "JT-TEST-001");
    assert_eq!(root_refs(&repo).len(), 3);
}

#[test]
fn a_replace_graft_does_not_hide_the_change_and_a_grafts_file_refuses() {
    let repo = blocking();
    repo.git(&["replace", "--graft", "HEAD"]);
    assert_eq!(repo.git(&["log", "--oneline", "HEAD"]).lines().count(), 1);
    let ran = jtac(&repo, &["check"]);
    assert_eq!(ran.code, 1, "{}{}", ran.stdout, ran.stderr);
    assert!(ran.stdout.contains("JT-TEST-001"), "{}", ran.stdout);
    assert!(root_refs(&repo).is_empty());
    let head = repo.git(&["rev-parse", "HEAD"]);
    repo.write(".git/info/grafts", format!("{head}\n").as_bytes());
    let ran = jtac(&repo, &["check"]);
    assert_eq!(ran.code, 2, "{}{}", ran.stdout, ran.stderr);
    assert!(
        ran.stderr
            .starts_with("jtac: the repository has a grafts file, `")
            && ran.stderr.contains("/.git/info/grafts`."),
        "{}",
        ran.stderr
    );
    assert!(
        ran.stderr
            .contains("/.git/info/grafts` (`git replace --convert-graft-file`"),
        "{}",
        ran.stderr
    );
}

#[test]
fn a_relative_signing_key_is_read_from_the_repository_root() {
    let repo = passing();
    repo.git(&["config", "jtac.signingkey", ".git/key"]);
    let ran = jtac_in(&repo, "src", &["check"]);
    assert_eq!(ran.code, 0, "{}{}", ran.stdout, ran.stderr);
    assert_eq!(root_refs(&repo).len(), 1);
}
