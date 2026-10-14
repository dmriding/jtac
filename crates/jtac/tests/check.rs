//! `jtac check --preview` against throwaway repositories: the verdict, the base rule, the
//! notes, and every refusal, through the built binary so the coverage floor measures `main`.
//! Policy checks run the binary itself (`--version` passes, `verify` is refused), so no check
//! reads a tool's real configuration.

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

fn failing(id: &str, required: bool) -> String {
    format!(
        "[checks.{id}]\nrun = [\"{}\", \"verify\"]\nrequired = {required}\n",
        me()
    )
}

struct Ran {
    code: i32,
    stdout: String,
    stderr: String,
}

fn jtac(repo: &Repo, args: &[&str]) -> Ran {
    jtac_with(repo, args, &[])
}

fn jtac_with(repo: &Repo, args: &[&str], env: &[(&str, &str)]) -> Ran {
    let out = repo.command_with(env!("CARGO_BIN_EXE_jtac"), args, env);
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

/// `main` holds a policy and one test; `feature` removes the test.
fn removal() -> Repo {
    let repo = Repo::init(&[]);
    repo.write(".jtac/policy.toml", policy("").as_bytes());
    repo.write("src/lib.rs", b"#[test]\nfn keeps() {}\nfn a() {}\n");
    repo.commit("base");
    repo.git(&["switch", "-q", "-c", "feature"]);
    repo.write("src/lib.rs", b"fn a() {}\n");
    repo.commit("tip");
    repo
}

fn rules(v: &Value) -> Vec<&str> {
    v["reasons"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|r| r["rule"].as_str())
        .collect()
}

fn base_from(repo: &Repo, args: &[&str]) -> (i32, String) {
    let (code, v) = json(repo, args);
    (code, v["base"]["from"].as_str().unwrap().to_owned())
}

#[test]
fn a_removed_test_aborts_with_exit_one_and_three_part_reasons() {
    let repo = removal();
    let ran = jtac(&repo, &["check", "--preview"]);
    assert_eq!(ran.code, 1, "{}", ran.stderr);
    assert!(ran.stderr.is_empty());
    let text = &ran.stdout;
    assert!(
        text.starts_with("ABORT (preview, nothing written): 1 reasons block"),
        "{text}"
    );
    assert!(text.contains("(merge base with main)"), "{text}");
    assert!(
        text.contains("1. JT-TEST-001 blocker src/lib.rs: test `keeps` removed"),
        "{text}"
    );
    assert!(text.contains("why  A removed test"), "{text}");
    assert!(text.contains("fix  restore the test"), "{text}");
    assert!(text.contains("--path src/lib.rs"), "{text}");
    assert!(text.contains("integrity pass"), "{text}");
    assert!(text.contains("ok pass"), "{text}");
    assert!(!text.contains("note"), "{text}");
    let (code, v) = json(&repo, &["check", "--preview", "--json"]);
    assert_eq!(code, 1);
    assert_eq!(v["v"], 1);
    assert_eq!(v["banner"], "ABORT");
    assert_eq!(v["pass"], false);
    assert_eq!(v["preview"], true);
    assert_eq!(rules(&v), ["JT-TEST-001"]);
    assert_eq!(v["reasons"][0]["kind"], "finding");
    assert_eq!(v["checks"][0]["id"], "integrity");
    assert_eq!(v["checks"][0]["tool_version"], env!("CARGO_PKG_VERSION"));
    assert_eq!(v["checks"][1]["argv"][1], "--version");
    assert!(v["commit"].as_str().unwrap().starts_with("sha1:"));
    assert!(v["policy"].as_str().unwrap().starts_with("sha256:"));
    assert_eq!(v["base"]["from"], "merge base with main");
    let wrote = repo.git(&["status", "--porcelain", "--", ".", ":(exclude)*.profraw"]);
    assert!(wrote.is_empty(), "preview wrote nothing: {wrote}");
}

#[test]
fn two_tests_removed_at_one_place_are_two_reasons_with_their_own_names() {
    let repo = Repo::init(&[]);
    repo.write(".jtac/policy.toml", policy("").as_bytes());
    repo.write(
        "src/lib.rs",
        b"#[test]\nfn alpha() {}\n#[test]\nfn beta() {}\n",
    );
    repo.commit("base");
    repo.write("src/lib.rs", b"fn a() {}\n");
    repo.commit("tip");
    let (_, v) = json(&repo, &["check", "--preview", "--json"]);
    let whats: Vec<&str> = v["reasons"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["what"].as_str().unwrap())
        .collect();
    assert_eq!(whats.len(), 2, "{whats:?}");
    assert!(whats.iter().any(|w| w.contains("`alpha`")), "{whats:?}");
    assert!(whats.iter().any(|w| w.contains("`beta`")), "{whats:?}");
}

#[test]
fn a_clean_change_clears_hot_and_counts_a_failed_optional_check_honestly() {
    let repo = Repo::init(&[]);
    repo.write(
        ".jtac/policy.toml",
        policy(&failing("advisory", false)).as_bytes(),
    );
    repo.write("src/lib.rs", b"fn a() {}\n");
    repo.commit("base");
    repo.git(&["switch", "-q", "-c", "feature"]);
    repo.write("src/lib.rs", b"fn a() {}\nfn b() {}\n");
    repo.commit("tip");
    let ran = jtac(&repo, &["check", "--preview"]);
    assert_eq!(ran.code, 0, "{}", ran.stdout);
    assert!(
        ran.stdout.starts_with(
            "CLEARED HOT (preview, nothing written): 2 of 3 checks pass, nothing blocks\n"
        ),
        "{}",
        ran.stdout
    );
    assert!(ran.stdout.contains("advisory fail"), "{}", ran.stdout);
    assert!(
        ran.stdout.contains("added   1 lines; tiers none"),
        "{}",
        ran.stdout
    );
    let (_, v) = json(&repo, &["check", "--preview", "--json"]);
    assert_eq!(v["state"], "covered");
    assert_eq!(v["added_lines"], 1);
    assert_eq!(v["tiers"], serde_json::json!([]));
    assert_eq!(v["reasons"].as_array().unwrap().len(), 0);
}

#[test]
fn the_base_is_the_flag_then_the_upstream_then_origin_head_then_main_then_master() {
    let repo = removal();
    let origin = Repo::empty();
    let bare = origin.path().join("origin.git");
    repo.git(&["init", "-q", "--bare", bare.to_str().unwrap()]);
    repo.git(&["remote", "add", "origin", bare.to_str().unwrap()]);
    repo.git(&["push", "-q", "origin", "main"]);
    assert_eq!(
        base_from(&repo, &["check", "--preview", "--json"]),
        (1, "merge base with main".into())
    );
    repo.git(&[
        "symbolic-ref",
        "refs/remotes/origin/HEAD",
        "refs/remotes/origin/main",
    ]);
    assert_eq!(
        base_from(&repo, &["check", "--preview", "--json"]),
        (1, "merge base with origin/main".into())
    );
    repo.git(&["push", "-q", "-u", "origin", "feature"]);
    let (code, from) = base_from(&repo, &["check", "--preview", "--json"]);
    assert_eq!(
        (code, from.as_str()),
        (
            1,
            "first parent of HEAD (merge base with origin/feature is at HEAD)"
        )
    );
    assert_eq!(
        base_from(&repo, &["check", "--preview", "--json", "--base", "main"]),
        (1, "--base main".into())
    );
    let (code, from) = base_from(&repo, &["check", "--preview", "--json", "--base", "HEAD"]);
    assert_eq!(
        (code, from.as_str()),
        (1, "first parent of HEAD (--base HEAD is at HEAD)")
    );
    let master = Repo::init(&["-b", "master"]);
    master.write(".jtac/policy.toml", policy("").as_bytes());
    master.commit("base");
    master.git(&["switch", "-q", "-c", "topic"]);
    master.write("a.txt", b"a\n");
    master.commit("tip");
    assert_eq!(
        base_from(&master, &["check", "--preview", "--json"]),
        (0, "merge base with master".into())
    );
}

#[test]
fn a_tag_named_main_does_not_shadow_the_branch() {
    let repo = removal();
    repo.write("src/more.rs", b"fn more() {}\n");
    repo.commit("more");
    repo.git(&["tag", "main", "HEAD~1"]);
    let (code, v) = json(&repo, &["check", "--preview", "--json"]);
    assert_eq!(code, 1, "{v}");
    assert_eq!(rules(&v), ["JT-TEST-001"]);
}

#[test]
fn with_no_candidate_or_no_common_ancestor_the_last_commit_is_judged() {
    let repo = Repo::init(&["-b", "trunk"]);
    repo.write(".jtac/policy.toml", policy("").as_bytes());
    repo.write("src/lib.rs", b"#[test]\nfn keeps() {}\n");
    repo.commit("only");
    let (code, from) = base_from(&repo, &["check", "--preview", "--json"]);
    assert_eq!(
        (code, from.as_str()),
        (
            0,
            "HEAD itself (HEAD has no parent, so there is no change to judge)"
        )
    );
    repo.write("src/lib.rs", b"fn a() {}\n");
    repo.commit("removes");
    let (code, v) = json(&repo, &["check", "--preview", "--json"]);
    assert_eq!(code, 1, "{v}");
    assert_eq!(
        v["base"]["from"],
        "first parent of HEAD (no upstream, no main or master branch)"
    );
    assert_eq!(rules(&v), ["JT-TEST-001"]);
    repo.git(&["switch", "-q", "--orphan", "main"]);
    repo.write(".jtac/policy.toml", policy("").as_bytes());
    repo.commit("unrelated main");
    repo.git(&["switch", "-q", "trunk"]);
    let (code, from) = base_from(&repo, &["check", "--preview", "--json"]);
    assert_eq!(
        (code, from.as_str()),
        (1, "first parent of HEAD (no common ancestor with main)")
    );
    let ran = jtac(&repo, &["check", "--preview", "--base", "main"]);
    assert_eq!(ran.code, 2);
    assert!(
        ran.stderr.starts_with("jtac: `--base main`: "),
        "{}",
        ran.stderr
    );
    assert!(
        ran.stderr
            .contains("negotiation: Run `jtac check --base HEAD~1`"),
        "{}",
        ran.stderr
    );
    let ran = jtac(&repo, &["check", "--preview", "--base", "nope"]);
    assert_eq!(ran.code, 2);
    assert!(ran.stdout.is_empty());
    assert!(
        ran.stderr.starts_with("jtac: `--base nope`: "),
        "{}",
        ran.stderr
    );
}

#[test]
fn a_failing_check_names_its_argv_and_each_note_has_its_own_cause() {
    let repo = Repo::init(&[]);
    repo.write(
        ".jtac/policy.toml",
        policy(&failing("bad", true)).as_bytes(),
    );
    repo.commit("base");
    let notes = |v: &Value| -> Vec<String> {
        v["notes"]
            .as_array()
            .unwrap()
            .iter()
            .map(|n| n.as_str().unwrap().to_owned())
            .collect()
    };
    let (code, v) = json(&repo, &["check", "--preview", "--json"]);
    assert_eq!(code, 1);
    let reason = &v["reasons"][0];
    assert_eq!(reason["kind"], "check");
    assert_eq!(reason["id"], "bad");
    assert_eq!(reason["what"], "check `bad` fail, exit 2");
    assert!(reason["why"].as_str().unwrap().contains("not `pass`"));
    assert_eq!(
        reason["how"],
        format!(
            "Run `{} verify` at the repository root and fix what it reports.",
            me()
        )
    );
    assert!(notes(&v).is_empty(), "{v}");
    repo.write("extra.txt", b"untracked\n");
    let (_, v) = json(&repo, &["check", "--preview", "--json"]);
    assert_eq!(notes(&v).len(), 1, "{v}");
    assert!(notes(&v)[0].contains("uncommitted"), "{v}");
    repo.remove("extra.txt");
    repo.write(".jtac/policy.toml", policy("").as_bytes());
    let (_, v) = json(&repo, &["check", "--preview", "--json"]);
    assert_eq!(notes(&v).len(), 2, "{v}");
    assert!(notes(&v)[1].contains("policy on disk differs"), "{v}");
    let ran = jtac(&repo, &["check", "--preview"]);
    assert!(
        ran.stdout.contains("note    the policy on disk differs"),
        "{}",
        ran.stdout
    );
}

#[test]
fn autocrlf_alone_is_not_a_policy_difference() {
    let repo = Repo::init(&[]);
    repo.write(".jtac/policy.toml", policy("").as_bytes());
    repo.commit("base");
    repo.git(&["config", "core.autocrlf", "true"]);
    let crlf = policy("").replace('\n', "\r\n");
    repo.write(".jtac/policy.toml", crlf.as_bytes());
    let (_, v) = json(&repo, &["check", "--preview", "--json"]);
    let notes = v["notes"].as_array().unwrap();
    assert!(
        notes
            .iter()
            .all(|n| !n.as_str().unwrap().contains("policy")),
        "{notes:?}"
    );
}

#[test]
fn missing_bad_and_unreadable_policies_refuse_with_exit_two() {
    let repo = Repo::init(&[]);
    repo.write("a.txt", b"a\n");
    repo.commit("no policy");
    let ran = jtac(&repo, &["check", "--preview"]);
    assert_eq!(ran.code, 2);
    assert!(ran.stdout.is_empty());
    assert!(
        ran.stderr
            .starts_with("jtac: HEAD has no `.jtac/policy.toml`.\n"),
        "{}",
        ran.stderr
    );
    assert!(ran.stderr.contains("negotiation: Write"), "{}", ran.stderr);
    repo.write(".jtac/policy.toml", b"version = 1\n[checks.ok]\nrun = []\n");
    repo.commit("bad policy");
    let ran = jtac(&repo, &["check", "--preview", "--json"]);
    assert_eq!(ran.code, 2);
    let v: Value = serde_json::from_str(&ran.stderr).unwrap();
    assert_eq!(
        v["error"]["what"],
        ".jtac/policy.toml at HEAD: checks.ok.run names no program"
    );
    assert!(
        v["error"]["how"].as_str().unwrap().contains("commit it"),
        "{v}"
    );
    assert_eq!(v["error"]["kind"], "negotiation");
    assert!(!v["error"]["how"].as_str().unwrap().is_empty());
    repo.write(".jtac/policy.toml", b"version = 1\n# \xff\n");
    repo.commit("not utf-8");
    let ran = jtac(&repo, &["check", "--preview"]);
    assert_eq!(ran.code, 2);
    assert!(ran.stderr.contains("not UTF-8"), "{}", ran.stderr);
}

#[cfg(unix)]
#[test]
fn a_policy_that_is_a_symlink_at_head_refuses() {
    let repo = Repo::init(&[]);
    repo.write("real.toml", policy("").as_bytes());
    std::fs::create_dir_all(repo.path().join(".jtac")).unwrap();
    std::os::unix::fs::symlink("../real.toml", repo.path().join(".jtac/policy.toml")).unwrap();
    repo.commit("linked");
    let ran = jtac(&repo, &["check", "--preview"]);
    assert_eq!(ran.code, 2);
    assert!(ran.stderr.contains("not a regular file"), "{}", ran.stderr);
    assert!(
        ran.stderr
            .contains("Make `.jtac/policy.toml` a regular file"),
        "{}",
        ran.stderr
    );
}

#[cfg(unix)]
#[test]
fn a_repository_path_with_a_trailing_space_is_read_as_written() {
    let outer = Repo::empty();
    let spaced = outer.path().join("repo ");
    std::fs::create_dir_all(&spaced).unwrap();
    let repo = Repo::init(&[]);
    let out = repo.command(
        "git",
        &["init", "-q", "-b", "main", spaced.to_str().unwrap()],
    );
    assert!(out.status.success());
    std::fs::create_dir_all(spaced.join(".jtac")).unwrap();
    std::fs::write(spaced.join(".jtac/policy.toml"), policy("")).unwrap();
    let git = |args: &[&str]| {
        let out = std::process::Command::new("git")
            .args(args)
            .current_dir(&spaced)
            .env_clear()
            .envs(repo.env())
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
    };
    git(&["add", "-A"]);
    git(&["commit", "-q", "--no-gpg-sign", "-m", "base"]);
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_jtac"))
        .args(["check", "--preview"])
        .current_dir(&spaced)
        .env_clear()
        .envs(repo.env())
        .output()
        .unwrap();
    assert_eq!(
        out.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn outside_a_repository_and_before_the_first_commit_refuse() {
    let none = Repo::empty();
    let ran = jtac(&none, &["check", "--preview"]);
    assert_eq!(ran.code, 2);
    assert!(
        ran.stderr
            .starts_with("jtac: `git rev-parse --show-toplevel` exited 128"),
        "{}",
        ran.stderr
    );
    assert!(
        ran.stderr.contains("negotiation: Run `git status`"),
        "{}",
        ran.stderr
    );
    let empty = Repo::init(&[]);
    let ran = jtac(&empty, &["check", "--preview"]);
    assert_eq!(ran.code, 2);
    assert!(
        ran.stderr
            .contains("negotiation: Make a commit on this branch"),
        "{}",
        ran.stderr
    );
}

#[test]
fn a_shallow_clone_refuses_instead_of_judging_nothing() {
    let origin = removal();
    let clone = Repo::empty();
    let dir = clone.path().join("shallow");
    let out = origin.command(
        "git",
        &[
            "clone",
            "-q",
            "--depth",
            "1",
            "-b",
            "feature",
            "--no-local",
            origin.path().to_str().unwrap(),
            dir.to_str().unwrap(),
        ],
    );
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_jtac"))
        .args(["check", "--preview"])
        .current_dir(&dir)
        .env_clear()
        .envs(origin.env())
        .output()
        .unwrap();
    assert_eq!(
        out.status.code(),
        Some(2),
        "{}",
        String::from_utf8_lossy(&out.stdout)
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("history is shallow"), "{stderr}");
    assert!(stderr.contains("git fetch --unshallow"), "{stderr}");
    let deeper = clone.path().join("depth2");
    origin.write("src/more.rs", b"fn more() {}\n");
    origin.commit("more");
    let out = origin.command(
        "git",
        &[
            "clone",
            "-q",
            "--depth",
            "2",
            "-b",
            "feature",
            "--no-local",
            origin.path().to_str().unwrap(),
            deeper.to_str().unwrap(),
        ],
    );
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let run = |args: &[&str]| {
        std::process::Command::new(env!("CARGO_BIN_EXE_jtac"))
            .args(args)
            .current_dir(&deeper)
            .env_clear()
            .envs(origin.env())
            .output()
            .unwrap()
    };
    let out = run(&["check", "--preview", "--base", "HEAD~1"]);
    assert_eq!(
        out.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let git = |args: &[&str]| {
        let out = std::process::Command::new("git")
            .args(args)
            .current_dir(&deeper)
            .env_clear()
            .envs(origin.env())
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
    };
    git(&[
        "config",
        "remote.origin.fetch",
        "+refs/heads/*:refs/remotes/origin/*",
    ]);
    git(&["fetch", "-q", "--depth", "1", "origin", "main"]);
    git(&["config", "branch.feature.merge", "refs/heads/main"]);
    let out = run(&["check", "--preview"]);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(2), "{stderr}");
    assert!(
        stderr.contains("merge base with origin/main is not in this clone"),
        "{stderr}"
    );
}

#[test]
fn a_directory_at_the_policy_path_on_disk_is_a_difference_not_a_refusal() {
    let repo = Repo::init(&[]);
    repo.write(".jtac/policy.toml", policy("").as_bytes());
    repo.commit("base");
    repo.remove(".jtac/policy.toml");
    std::fs::create_dir(repo.path().join(".jtac/policy.toml")).unwrap();
    let (code, v) = json(&repo, &["check", "--preview", "--json"]);
    assert_eq!(code, 0, "{v}");
    let notes = v["notes"].as_array().unwrap();
    assert!(
        notes
            .iter()
            .any(|n| n.as_str().unwrap().contains("policy on disk differs")),
        "{notes:?}"
    );
}

#[cfg(unix)]
#[test]
fn a_failing_clean_filter_on_the_policy_refuses_instead_of_noting_a_difference() {
    let repo = Repo::init(&[]);
    repo.write(".jtac/policy.toml", policy("").as_bytes());
    repo.commit("base");
    repo.write(".gitattributes", b".jtac/policy.toml filter=broken\n");
    repo.git(&["config", "filter.broken.clean", "false"]);
    repo.git(&["config", "filter.broken.required", "true"]);
    let ran = jtac(&repo, &["check", "--preview"]);
    assert_eq!(ran.code, 2, "{}", ran.stdout);
    assert!(ran.stderr.contains("hash-object"), "{}", ran.stderr);
}

#[test]
fn a_candidate_pointing_at_a_missing_object_refuses_instead_of_falling_back() {
    let repo = removal();
    repo.write("src/more.rs", b"fn more() {}\n");
    repo.commit("more");
    repo.write(
        ".git/refs/heads/main",
        b"0000000000000000000000000000000000000001\n",
    );
    let ran = jtac(&repo, &["check", "--preview"]);
    assert_eq!(ran.code, 2, "{}", ran.stdout);
    assert!(ran.stderr.contains("merge-base"), "{}", ran.stderr);
}

#[test]
fn a_policy_check_that_runs_jtac_refuses_inside_instead_of_recursing() {
    let repo = Repo::init(&[]);
    let policy = format!(
        "version = 1\n[checks.self]\nrun = [\"{}\", \"check\", \"--preview\"]\n",
        me()
    );
    repo.write(".jtac/policy.toml", policy.as_bytes());
    repo.commit("recursive");
    let (code, v) = json(&repo, &["check", "--preview", "--json"]);
    assert_eq!(code, 1, "{v}");
    assert_eq!(v["reasons"][0]["id"], "self");
    assert_eq!(v["checks"][1]["exit"], 2);
    let ran = jtac_with(&repo, &["check", "--preview"], &[("JTAC_CHECK", "1")]);
    assert_eq!(ran.code, 2);
    assert!(
        ran.stderr.contains("`JTAC_CHECK=1` is set"),
        "{}",
        ran.stderr
    );
    let ran = jtac_with(&repo, &["check", "--preview"], &[("JTAC_CHECK", "0")]);
    assert_eq!(ran.code, 1, "{}", ran.stderr);
}

#[test]
fn version_help_and_json_refusals_work_on_the_binary() {
    let repo = Repo::empty();
    let ran = jtac(&repo, &["--version"]);
    assert_eq!(
        (ran.code, ran.stdout.trim()),
        (0, format!("jtac {}", env!("CARGO_PKG_VERSION")).as_str())
    );
    let ran = jtac(&repo, &["--help"]);
    assert_eq!(ran.code, 0);
    assert!(
        ran.stdout.contains("check") && ran.stderr.is_empty(),
        "{}",
        ran.stdout
    );
    for args in [
        &["status", "--json"][..],
        &["check", "--json"],
        &["check", "--preview", "--json", "--nope"],
    ] {
        let ran = jtac(&repo, args);
        assert_eq!(ran.code, 2, "{args:?}");
        let v: Value = serde_json::from_str(&ran.stderr)
            .unwrap_or_else(|_| panic!("{args:?}: {}", ran.stderr));
        assert_eq!(v["v"], 1);
        assert_eq!(v["error"]["kind"], "negotiation", "{args:?}");
    }
}
