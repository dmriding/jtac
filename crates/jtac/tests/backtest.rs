//! `jtac backtest` on a scripted history (DESIGN.md section 11.10, decision 51): the units a
//! merge, a squash-style commit and a root give, a removed test under the default policy,
//! `preexisting` on a file already past the cap, a summary that names nothing from the tree,
//! the policy of the range's newest commit named by hash, a rows file created once, the
//! refusals with exit 2 naming the policy they read, and a run that writes nothing.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use jtac_testkit::Repo;
use serde_json::{Value, json};

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

fn summary(repo: &Repo, args: &[&str]) -> Value {
    let mut args = args.to_vec();
    args.push("--json");
    let ran = jtac(repo, &args);
    assert_eq!((ran.code, ran.stderr.as_str()), (0, ""), "{}", ran.stdout);
    serde_json::from_str(&ran.stdout).unwrap()
}

fn rows(path: &Path) -> Vec<Value> {
    std::fs::read_to_string(path)
        .unwrap()
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect()
}

fn arg(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

fn lines(prefix: &str, n: usize) -> String {
    (0..n)
        .map(|i| format!("pub fn {prefix}{i}() {{}}\n"))
        .collect()
}

const TESTS: &str = "#[test]\nfn checks_the_sum() {\n    assert_eq!(1 + 2, 3);\n}\n";
const MORE: &str = "\n#[test]\nfn checks_the_difference() {\n    assert_eq!(3 - 2, 1);\n}\n";

/// The commits of [`history`], newest last.
struct Ids {
    root: String,
    edit: String,
    side: String,
    merge: String,
    grow: String,
    remove: String,
}

/// A root holding the policy (file cap 6) and a file already past the cap; a one-line edit to
/// it; a side branch merged with `--no-ff`; a single-parent commit pushing a file past the
/// cap; a commit removing a test.
fn history() -> (Repo, Ids) {
    let repo = Repo::init(&[]);
    repo.write(
        ".jtac/policy.toml",
        b"version = 1\n[integrity]\nmax_file_lines = 6\n",
    );
    repo.write("src/big.rs", lines("b", 8).as_bytes());
    repo.write("src/grow.rs", lines("g", 5).as_bytes());
    repo.write("tests/sum.rs", format!("{TESTS}{MORE}").as_bytes());
    let root = repo.commit("root");
    repo.write("src/big.rs", lines("b", 8).replace("b3", "c3").as_bytes());
    let edit = repo.commit("edit big");
    repo.git(&["checkout", "-q", "-b", "side"]);
    repo.write("src/feature.rs", lines("f", 2).as_bytes());
    let side = repo.commit("feature");
    repo.git(&["checkout", "-q", "main"]);
    repo.git(&[
        "merge",
        "-q",
        "--no-ff",
        "--no-gpg-sign",
        "-m",
        "merge",
        "side",
    ]);
    let merge = repo.git(&["rev-parse", "HEAD"]);
    repo.write("src/grow.rs", lines("g", 8).as_bytes());
    let grow = repo.commit("grow");
    repo.write("tests/sum.rs", TESTS.as_bytes());
    let remove = repo.commit("remove a test");
    let ids = Ids {
        root,
        edit,
        side,
        merge,
        grow,
        remove,
    };
    (repo, ids)
}

/// A directory outside the repository for `--policy` and `--rows`.
fn outside() -> (Repo, PathBuf) {
    let dir = Repo::empty();
    let rows = dir.path().join("rows.jsonl");
    (dir, rows)
}

fn commit_of(row: &Value) -> String {
    row["commit"]
        .as_str()
        .unwrap()
        .rsplit(':')
        .next()
        .unwrap()
        .to_owned()
}

#[test]
fn the_first_parent_line_gives_one_unit_per_merge_squash_and_commit_and_skips_the_root() {
    let (repo, ids) = history();
    let (_dir, path) = outside();
    let s = summary(&repo, &["backtest", "main", "--rows", &arg(&path)]);
    assert_eq!(
        (s["units"].as_u64(), s["skipped"].as_u64()),
        (Some(4), Some(1))
    );
    assert_eq!(s["rows_written"], 4);
    let rows = rows(&path);
    let got: Vec<(String, u64, u64)> = rows
        .iter()
        .map(|r| {
            let n = |k: &str| r[k].as_u64().unwrap();
            (commit_of(r), n("parents"), n("paths"))
        })
        .collect();
    assert_eq!(
        got,
        [
            (ids.remove, 1, 1),
            (ids.grow, 1, 1),
            (ids.merge, 2, 1),
            (ids.edit, 1, 1),
        ]
    );
    assert!(
        !rows
            .iter()
            .any(|r| commit_of(r) == ids.side || commit_of(r) == ids.root),
        "the side commit is the merge's, and the root has nothing to judge"
    );
    assert!(rows[2]["findings"].as_array().unwrap().is_empty());
}

#[test]
fn a_removed_test_blocks_under_the_default_policy_and_still_exits_zero() {
    let (repo, ids) = history();
    let (dir, path) = outside();
    let policy = dir.write("policy.toml", b"version = 1\n");
    let s = summary(
        &repo,
        &[
            "backtest",
            "main",
            "--policy",
            &arg(&policy),
            "--rows",
            &arg(&path),
        ],
    );
    assert_eq!(
        (s["blocking"].as_u64(), s["blocking_per_mille"].as_u64()),
        (Some(1), Some(250))
    );
    assert_eq!(s["block_at"], "major");
    assert_eq!(
        s["findings"],
        json!([{"rule": "JT-TEST-001", "severity": "blocker", "findings": 1}])
    );
    assert_eq!(
        s["blocked_alone"],
        json!([{"rule": "JT-TEST-001", "units": 1}])
    );
    let rows = rows(&path);
    let removed = rows.iter().find(|r| commit_of(r) == ids.remove).unwrap();
    assert_eq!(removed["blocks"], true);
    assert_eq!(removed["findings"][0]["rule"], "JT-TEST-001");
    assert_eq!(removed["findings"][0]["severity"], "blocker");
    assert_eq!(removed["findings"][0]["path"], "tests/sum.rs");
    assert!(removed["findings"][0].get("preexisting").is_none());
    assert_eq!(
        jtac(&repo, &["backtest", "main"]).code,
        0,
        "a report never exits 1"
    );
}

#[test]
fn a_file_already_past_the_cap_is_preexisting_and_one_pushed_past_it_is_not() {
    let (repo, ids) = history();
    let (_dir, path) = outside();
    let s = summary(&repo, &["backtest", "main", "--rows", &arg(&path)]);
    let size = |id: &str| -> Value {
        let rows = rows(&path);
        let row = rows.iter().find(|r| commit_of(r) == id).unwrap().clone();
        assert_eq!(row["findings"][0]["rule"], "JT-SIZE-001", "{row}");
        row["findings"][0].clone()
    };
    assert_eq!(size(&ids.edit)["preexisting"], true);
    assert_eq!(size(&ids.edit)["path"], "src/big.rs");
    assert_eq!(size(&ids.grow)["preexisting"], false);
    assert_eq!(
        s["findings"],
        json!([
            {"rule": "JT-TEST-001", "severity": "blocker", "findings": 1},
            {"rule": "JT-SIZE-001", "severity": "major", "findings": 2, "preexisting": 1},
        ])
    );
    assert_eq!(s["languages"][0]["language"], "rust");
    assert_eq!(s["languages"][0]["findings"], s["findings"]);
    assert_eq!(
        (s["blocking"].as_u64(), s["silent"].as_u64()),
        (Some(3), Some(1))
    );
}

#[test]
fn the_summary_names_no_path_message_commit_or_author() {
    let (repo, ids) = history();
    let (_dir, path) = outside();
    let text = jtac(&repo, &["backtest", "main", "--rows", &arg(&path)]);
    assert_eq!(text.code, 0, "{}", text.stderr);
    assert!(text.stdout.contains("JT-SIZE-001"), "{}", text.stdout);
    assert!(
        text.stdout.contains("holds paths and messages"),
        "{}",
        text.stdout
    );
    assert!(
        text.stdout.contains("what this does not say"),
        "{}",
        text.stdout
    );
    let json = jtac(&repo, &["backtest", "main", "--json"]).stdout;
    let mut secrets: Vec<String> = [
        "big.rs",
        "grow.rs",
        "sum.rs",
        "feature.rs",
        "policy.toml",
        "src/",
        "tests/",
        "checks_the",
        "production lines",
        "rows.jsonl",
        "example",
    ]
    .map(str::to_owned)
    .to_vec();
    for id in [
        ids.root, ids.edit, ids.side, ids.merge, ids.grow, ids.remove,
    ] {
        secrets.push(id[..7].to_owned());
    }
    for out in [&text.stdout, &json] {
        for s in &secrets {
            assert!(!out.contains(s.as_str()), "{s:?} leaked into\n{out}");
        }
    }
    assert!(
        std::fs::read_to_string(&path)
            .unwrap()
            .contains("src/big.rs"),
        "the rows file is where paths go"
    );
}

#[test]
fn rows_file_is_created_and_never_replaced() {
    let (repo, _) = history();
    let (_dir, path) = outside();
    assert_eq!(
        jtac(&repo, &["backtest", "main", "--rows", &arg(&path)]).code,
        0
    );
    let before = std::fs::read(&path).unwrap();
    assert_eq!(rows(&path).len(), 4);
    let again = jtac(&repo, &["backtest", "main", "--rows", &arg(&path)]);
    assert_eq!((again.code, again.stdout.as_str()), (2, ""));
    assert!(again.stderr.starts_with("jtac: "), "{}", again.stderr);
    assert!(
        again.stderr.contains("never replaces one"),
        "{}",
        again.stderr
    );
    assert!(
        again
            .stderr
            .contains("\nnegotiation: Name a file that does not exist yet: `jtac backtest --rows"),
        "{}",
        again.stderr
    );
    assert_eq!(std::fs::read(&path).unwrap(), before);
}

#[test]
fn no_policy_refuses_in_three_parts_with_exit_two() {
    let repo = Repo::init(&[]);
    repo.write("src/a.rs", b"pub fn a() {}\n");
    repo.commit("root");
    repo.write("src/a.rs", b"pub fn b() {}\n");
    repo.commit("edit");
    repo.write(".jtac/policy.toml", b"version = 1\n");
    let ran = jtac(&repo, &["backtest", "HEAD"]);
    assert_eq!((ran.code, ran.stdout.as_str()), (2, ""));
    let parts: Vec<&str> = ran.stderr.lines().collect();
    assert_eq!(parts.len(), 3, "{}", ran.stderr);
    assert!(
        parts[0].starts_with("jtac: no policy to backtest `HEAD`"),
        "{}",
        parts[0]
    );
    assert!(parts[1].contains("decision 26"), "{}", parts[1]);
    assert!(
        parts[2].starts_with("negotiation: Run `jtac init --dry-run`")
            && parts[2].contains("jtac backtest --policy ../backtest-policy.toml HEAD"),
        "{}",
        parts[2]
    );
    let json = jtac(&repo, &["backtest", "HEAD", "--json"]);
    assert_eq!(json.code, 2);
    let v: Value = serde_json::from_str(&json.stderr).unwrap();
    assert!(
        v["error"]["how"]
            .as_str()
            .unwrap()
            .contains("jtac init --dry-run")
    );
    let missing = jtac(
        &repo,
        &["backtest", "HEAD", "--policy", "no-such-policy.toml"],
    );
    assert_eq!(missing.code, 2, "{}", missing.stderr);
    assert!(
        missing.stderr.contains("could not be read"),
        "{}",
        missing.stderr
    );
}

fn finds(summary: &Value, rule: &str) -> bool {
    summary["findings"]
        .as_array()
        .unwrap()
        .iter()
        .any(|f| f["rule"] == rule)
}

#[test]
fn the_policy_is_the_newest_commit_s_of_the_range_and_the_summary_names_it() {
    let loose = b"version = 1\n[integrity]\nmax_file_lines = 100\n";
    let strict = "version = 1\n[integrity]\nmax_file_lines = 6\n";
    let repo = Repo::init(&[]);
    repo.write(".jtac/policy.toml", loose);
    repo.write("src/a.rs", lines("a", 4).as_bytes());
    repo.commit("root, loose");
    repo.write(".jtac/policy.toml", strict.as_bytes());
    repo.write("src/a.rs", lines("a", 8).as_bytes());
    repo.commit("strict, and a file past its cap");
    repo.write(".jtac/policy.toml", loose);
    repo.write("src/a.rs", lines("a", 9).as_bytes());
    repo.commit("loose again");

    let below = summary(&repo, &["backtest", "HEAD~1"]);
    assert!(
        finds(&below, "JT-SIZE-001"),
        "the cap of the range's newest commit, not the root's or HEAD's: {below}"
    );
    assert_eq!(below["policy_from"], "newest-commit");
    let head = summary(&repo, &["backtest", "HEAD"]);
    assert!(!finds(&head, "JT-SIZE-001"), "{head}");
    assert_ne!(head["policy"], below["policy"]);
    assert!(
        head["policy"].as_str().unwrap().starts_with("sha256:"),
        "{head}"
    );

    let (dir, _) = outside();
    let own = dir.path().join("own.toml");
    std::fs::write(&own, strict).unwrap();
    let pinned = summary(&repo, &["backtest", "HEAD", "--policy", &arg(&own)]);
    assert_eq!(pinned["policy_from"], "file");
    assert_eq!(pinned["policy"], below["policy"], "one policy, one hash");
    assert!(finds(&pinned, "JT-SIZE-001"), "{pinned}");
    let text = jtac(&repo, &["backtest", "HEAD"]).stdout;
    let line = format!(
        "policy       {} from the range's newest commit",
        head["policy"].as_str().unwrap()
    );
    assert!(text.contains(&line), "{text}");
}

#[test]
fn a_refused_policy_names_where_it_was_read_and_never_the_gate_s_file_or_command() {
    let (repo, _) = history();
    let (dir, _) = outside();
    let own = dir.path().join("own.toml");
    std::fs::write(&own, "version = 1\nunknown = 1\n").unwrap();
    let file = jtac(&repo, &["backtest", "main", "--policy", &arg(&own)]);
    assert_eq!((file.code, file.stdout.as_str()), (2, ""));
    assert!(
        file.stderr.contains("the `--policy` file was refused: ")
            && file.stderr.contains("line 2, column 1")
            && file.stderr.contains("then run `jtac backtest --policy "),
        "{}",
        file.stderr
    );
    assert!(
        !file.stderr.contains(".jtac/policy.toml"),
        "{}",
        file.stderr
    );

    repo.write(".jtac/policy.toml", b"version = 1\nunknown = 1\n");
    let bad = repo.commit("a policy this build refuses");
    let committed = jtac(&repo, &["backtest", "main"]);
    assert_eq!((committed.code, committed.stdout.as_str()), (2, ""));
    for want in [
        format!("the range's newest commit `{bad}` holds a policy this build refuses: "),
        format!("`git show {bad}:.jtac/policy.toml` to `../backtest-policy.toml`"),
        "edit ../backtest-policy.toml line 2, column 1".to_owned(),
        "jtac backtest --policy ../backtest-policy.toml main".to_owned(),
    ] {
        assert!(
            committed.stderr.contains(&want),
            "{want:?} in {}",
            committed.stderr
        );
    }

    #[cfg(unix)]
    let linked = {
        std::fs::remove_file(repo.path().join(".jtac/policy.toml")).unwrap();
        std::os::unix::fs::symlink("elsewhere.toml", repo.path().join(".jtac/policy.toml"))
            .unwrap();
        let link = repo.commit("the policy is a link");
        let linked = jtac(&repo, &["backtest", "main"]);
        assert_eq!((linked.code, linked.stdout.as_str()), (2, ""));
        let want = format!(
            "`.jtac/policy.toml` in the range's newest commit `{link}` could not be read: it is \
             a link or a directory there."
        );
        assert!(linked.stderr.contains(&want), "{}", linked.stderr);
        assert!(!linked.stderr.contains("HEAD"), "{}", linked.stderr);
        linked.stderr
    };
    #[cfg(not(unix))]
    let linked = String::new();
    for stderr in [&file.stderr, &committed.stderr, &linked] {
        assert!(
            !stderr.contains("jtac check"),
            "the gate's command: {stderr}"
        );
    }
}

#[test]
fn an_unreadable_range_and_a_directory_outside_git_exit_two() {
    let (repo, _) = history();
    let (dir, _) = outside();
    let bad = jtac(&repo, &["backtest", "nope..main"]);
    assert_eq!((bad.code, bad.stdout.as_str()), (2, ""));
    assert!(
        bad.stderr.contains("not a range git can list"),
        "{}",
        bad.stderr
    );
    let planted = dir.path().join("planted");
    let option = format!("--output={}", arg(&planted));
    let ran = jtac(&repo, &["backtest", "--", &option]);
    assert_eq!(ran.code, 2, "{}", ran.stderr);
    assert!(
        !planted.exists(),
        "an option-shaped range reached git as an option"
    );
    let nowhere = jtac(&dir, &["backtest", "HEAD"]);
    assert_eq!((nowhere.code, nowhere.stdout.as_str()), (2, ""));
}

#[test]
fn a_shallow_boundary_refuses_rather_than_reading_as_a_root() {
    let (repo, ids) = history();
    repo.write(".git/shallow", format!("{}\n", ids.edit).as_bytes());
    let ran = jtac(&repo, &["backtest", "main"]);
    assert_eq!(ran.code, 2, "{}", ran.stdout);
    assert!(
        ran.stderr.contains("the history is shallow"),
        "{}",
        ran.stderr
    );
    assert!(
        ran.stderr.contains("git fetch --unshallow"),
        "{}",
        ran.stderr
    );
}

/// Every file under `.git`, by path, with its bytes.
fn snapshot(dir: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    let mut files = BTreeMap::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        for entry in std::fs::read_dir(&d).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                stack.push(path);
            } else {
                files.insert(path.clone(), std::fs::read(&path).unwrap());
            }
        }
    }
    files
}

#[test]
fn a_clone_that_never_ran_init_backtests_and_nothing_is_written() {
    let (origin, _) = history();
    let clone = Repo::clone_of(&origin);
    let git_dir = clone.path().join(".git");
    let refs = clone.git(&["for-each-ref"]);
    let before = snapshot(&git_dir);
    let text = jtac(&clone, &["backtest", "origin/main"]);
    assert_eq!(text.code, 0, "{}", text.stderr);
    assert!(
        text.stdout
            .starts_with("BACKTEST: 4 units judged, 1 unit skipped"),
        "{}",
        text.stdout
    );
    assert_eq!(summary(&clone, &["backtest", "origin/main"])["units"], 4);
    assert_eq!(snapshot(&git_dir), before, "backtest wrote under .git");
    assert_eq!(clone.git(&["status", "--porcelain", "--ignored"]), "");
    assert_eq!(clone.git(&["for-each-ref"]), refs);
    assert!(!git_dir.join("jtac").exists());
    assert!(!git_dir.join("hooks").join("pre-push").exists());
}
