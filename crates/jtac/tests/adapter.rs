//! The GitHub adapter's default-branch job (DESIGN.md section 15.2, decision 67): the
//! scripts of `adapter/github/` run as the action runs them, against a bare repository for
//! the forge's git and a stub `gh` that answers the forge's API from files.
#![cfg(unix)]

use std::path::{Path, PathBuf};
use std::process::Command;

use jtac_core::{Branch, GitOid, Pin, Results, Selected, select};
use jtac_testkit::Repo;
use serde_json::{Value, json};

const WORKFLOW: &str = ".github/workflows/ci.yml";
const SLUG: &str = "repos/example-org/example-app";
const CI_OPTION: &str = "namespaces=\"jtac-ci-root-v1,jtac-release-v1\" ";
const ROOTS: &str = "refs/jtac/roots/*:refs/jtac/roots/*";
const JTAC: &str = "+refs/jtac/*:refs/jtac/*";
const ZERO: &str = "0000000000000000000000000000000000000000";
/// `gh api [--paginate] <path> [--jq <expr>]` from the file the path names.
const GH: &str = r#"#!/bin/sh
[ "$1" = api ] || exit 64
shift
path=""; expr="."
while [ $# -gt 0 ]; do
  case "$1" in
    --paginate) ;;
    --jq) shift; expr=$1 ;;
    *) path=$1 ;;
  esac
  shift
done
file="$GH_FIXTURES/$(printf '%s' "$path" | tr -c 'A-Za-z0-9' '_').json"
[ -f "$file" ] || { echo "stub gh: no fixture for $path" >&2; exit 1; }
exec jq -r "$expr" "$file"
"#;

fn me() -> String {
    env!("CARGO_BIN_EXE_jtac").to_owned()
}

fn adapter(file: &str) -> String {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../adapter/github");
    root.join(file).to_string_lossy().into_owned()
}

fn key(dir: &Path, name: &str, email: &str) -> PathBuf {
    let key = dir.join(".git").join(name);
    let out = Command::new("ssh-keygen")
        .args(["-q", "-t", "ed25519", "-N", "", "-C", email, "-f"])
        .arg(&key)
        .output()
        .unwrap();
    assert!(out.status.success());
    key
}

fn signers_line(key: &Path, email: &str, option: &str) -> String {
    let pubkey = std::fs::read_to_string(key.with_extension("pub")).unwrap();
    let pubkey: Vec<&str> = pubkey.split_whitespace().take(2).collect();
    format!("{email} {option}{}\n", pubkey.join(" "))
}

fn ok(repo: &Repo, args: &[&str]) -> String {
    let out = repo.command(&me(), args);
    let text = String::from_utf8_lossy(&out.stdout).into_owned();
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "{args:?}: {text}{err}");
    text
}

/// What a script printed: its exit code, and the step summary with stderr after it.
struct Ran {
    code: i32,
    text: String,
}

/// A run as GitHub lists it, with one job named `e2e`.
struct Run<'a> {
    id: u64,
    workflow: &'a str,
    status: &'a str,
    conclusion: Option<&'a str>,
}

impl Run<'_> {
    fn of(conclusion: Option<&str>) -> Run<'_> {
        Run {
            id: 7,
            workflow: WORKFLOW,
            status: conclusion.map_or("in_progress", |_| "completed"),
            conclusion,
        }
    }
}

/// `origin` is the forge's repository, `dev` a developer's clone, `ci` the job's checkout,
/// `gh` the stub's files and the runner's temp dir. The policy reads `e2e` from
/// `WORKFLOW` and runs `ok` itself; `main` is the adoption commit, pushed directly.
struct Forge {
    origin: Repo,
    dev: Repo,
    ci: Repo,
    gh: Repo,
    ci_key: PathBuf,
    main: String,
}

fn forge() -> Forge {
    let origin = Repo::bare(&[]);
    let dev = Repo::clone_of(&origin);
    let policy = format!(
        "version = 2\n[checks.ok]\nrun = [\"{}\", \"--version\"]\n\
         [checks.e2e]\nci = {{ workflow = \"{WORKFLOW}\" }}\n",
        me()
    );
    dev.write(".jtac/policy.toml", policy.as_bytes());
    dev.write(WORKFLOW, b"on: push\n");
    dev.write(".gitignore", b"*.profraw\n");
    let dev_key = key(dev.path(), "dev", "dev@example.com");
    let ci_key = key(dev.path(), "ci", "ci@example.com");
    let lines = format!(
        "{}{}",
        signers_line(&dev_key, "dev@example.com", ""),
        signers_line(&ci_key, "ci@example.com", CI_OPTION)
    );
    dev.write(".jtac/signers", lines.as_bytes());
    dev.git(&["config", "jtac.signingkey", &dev_key.to_string_lossy()]);
    dev.git(&["config", "user.email", "dev@example.com"]);
    let main = dev.commit("adopt");
    ok(&dev, &["check"]);
    dev.git(&["push", "-q", "-u", "origin", "main"]);
    dev.git(&["push", "-q", "origin", ROOTS]);
    let ci = Repo::clone_of(&origin);
    ci.git(&["fetch", "-q", "origin", JTAC]);
    let gh = Repo::empty();
    let stub = gh.write("gh", GH.as_bytes());
    let out = Command::new("chmod").arg("+x").arg(&stub).output().unwrap();
    assert!(out.status.success());
    std::fs::create_dir(gh.path().join("tmp")).unwrap();
    Forge {
        origin,
        dev,
        ci,
        gh,
        ci_key,
        main,
    }
}

impl Forge {
    /// Pull request `number`: a branch of one commit off `origin/main` with the developer's
    /// root, merged by the forge with a merge commit. Returns the head and the merge.
    fn merged(&self, number: u32) -> (String, String) {
        self.land(number, true, &|dev| {
            dev.write(&format!("pr{number}.txt"), b"x\n");
        })
    }

    /// `merged` with the head's change given, and no root for it unless `rooted`.
    fn land(&self, number: u32, rooted: bool, change: &dyn Fn(&Repo)) -> (String, String) {
        let name = format!("pr{number}");
        self.dev.git(&["fetch", "-q", "origin"]);
        self.dev.git(&["switch", "-q", "-c", &name, "origin/main"]);
        change(&self.dev);
        let head = self.dev.commit(&name);
        if rooted {
            ok(&self.dev, &["check", "--base", "origin/main"]);
        }
        let pull = format!("{head}:refs/pull/{number}/head");
        self.dev.git(&["push", "-q", "origin", &name, &pull, ROOTS]);
        self.ci.git(&["fetch", "-q", "origin"]);
        self.ci
            .git(&["checkout", "-q", "-B", "main", "origin/main"]);
        let merge = ["merge", "-q", "--no-ff", "-m", "merge"];
        self.ci
            .git(&[&merge[..], &[&format!("origin/{name}")]].concat());
        let tip = self.ci.git(&["rev-parse", "HEAD"]);
        self.ci.git(&["push", "-q", "origin", "main"]);
        self.fixture(
            &format!("{SLUG}/commits/{tip}/pulls"),
            &json!([{
                "number": number,
                "merged_at": "2026-01-01T00:00:00Z",
                "merge_commit_sha": tip,
                "head": {"sha": head, "repo": {"clone_url": null}},
            }]),
        );
        (head, tip)
    }

    /// What the stub answers for `path`.
    fn fixture(&self, path: &str, body: &Value) {
        let name: String = path
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
            .collect();
        self.gh
            .write(&format!("{name}.json"), body.to_string().as_bytes());
    }

    /// The forge's push list for `main`, newest first: `(before, after, activity_type)`.
    fn pushes(&self, rows: &[(&str, &str, &str)]) {
        let body: Vec<Value> = rows
            .iter()
            .map(|(before, after, kind)| json!({"before": before, "after": after, "activity_type": kind}))
            .collect();
        let path = format!("{SLUG}/activity?ref=refs/heads/main&direction=desc&per_page=30");
        self.fixture(&path, &Value::from(body));
    }

    /// The runs the forge lists for `commit`, each a push to `main`.
    fn runs(&self, commit: &str, runs: &[Run]) {
        let listed: Vec<Value> = runs
            .iter()
            .map(|r| {
                json!({
                    "id": r.id, "path": r.workflow, "head_sha": commit, "event": "push",
                    "head_branch": "main", "run_attempt": 1, "status": r.status,
                    "conclusion": r.conclusion,
                })
            })
            .collect();
        let path = format!("{SLUG}/actions/runs?head_sha={commit}&per_page=100");
        self.fixture(&path, &json!({"workflow_runs": listed}));
        for r in runs {
            let job =
                json!({"id": r.id * 10, "run_id": r.id, "name": "e2e", "conclusion": r.conclusion});
            let path = format!(
                "{SLUG}/actions/runs/{}/jobs?filter=latest&per_page=100",
                r.id
            );
            self.fixture(&path, &json!({"jobs": [job]}));
        }
    }

    fn script(&self, file: &str, more: &[(&str, &str)]) -> Ran {
        let tmp = self.gh.path().join("tmp").to_string_lossy().into_owned();
        let summary = format!("{tmp}/summary");
        let output = format!("{tmp}/output");
        std::fs::write(&summary, "").unwrap();
        std::fs::write(&output, "").unwrap();
        let (gh, fixtures) = (self.gh.path().join("gh"), self.gh.path());
        let (gh, fixtures) = (gh.to_string_lossy(), fixtures.to_string_lossy());
        let (bin, dir) = (me(), adapter(""));
        let env = [
            ("GITHUB_REPOSITORY", "example-org/example-app"),
            ("GITHUB_STEP_SUMMARY", summary.as_str()),
            ("GITHUB_OUTPUT", output.as_str()),
            ("RUNNER_TEMP", tmp.as_str()),
            ("JTAC_BIN", bin.as_str()),
            ("JTAC_ADAPTER", dir.as_str()),
            ("GH", &*gh),
            ("GH_FIXTURES", &*fixtures),
            ("REMOTE", "origin"),
            ("BRANCH", "main"),
        ];
        let out = self.ci.command_with(
            "bash",
            &[adapter(file).as_str()],
            &[&env[..], more].concat(),
        );
        let rung = std::fs::read_to_string(&output).unwrap();
        let said = std::fs::read_to_string(&summary).unwrap();
        Ran {
            code: out.status.code().unwrap_or(-1),
            text: format!("{rung}{said}{}", String::from_utf8_lossy(&out.stderr)),
        }
    }

    /// The doorbell for a completed run.
    fn bell(&self, event: &str, branch: &str, sha: &str) -> Ran {
        let run = [
            ("RUN_EVENT", event),
            ("RUN_BRANCH", branch),
            ("RUN_SHA", sha),
        ];
        self.script("bell.sh", &run)
    }

    /// The default-branch job after a workflow completed for `sha`, holding the CI key.
    fn drain(&self, sha: &str) -> Ran {
        let key = std::fs::read_to_string(&self.ci_key).unwrap();
        let job = [
            ("GITHUB_EVENT_NAME", "workflow_run"),
            ("RUN_SHA", sha),
            ("KEY", key.trim_end()),
            ("PRINCIPAL", "ci@example.com"),
        ];
        let ran = self.script("root.sh", &job);
        let left = self.gh.path().join("tmp/jtac-ci-key");
        assert!(!left.exists(), "the job left the key behind");
        ran
    }

    /// The CI gate's reading of `commit` in a second clone, and its root's first parent.
    fn reads(&self, commit: &str) -> (String, String) {
        let second = Repo::clone_of(&self.origin);
        second.git(&["fetch", "-q", "origin", JTAC]);
        let v: Value =
            serde_json::from_str(&ok(&second, &["audit", &format!("{commit}^!"), "--json"]))
                .unwrap();
        let ci = v["rows"][0]["ci"].as_str().unwrap_or("none").to_owned();
        let name = format!("refs/jtac/roots/{}/{commit}:root.json", &commit[..2]);
        let out = second.command("git", &["cat-file", "blob", &name]);
        let root: Value = serde_json::from_slice(&out.stdout).unwrap_or(Value::Null);
        let parent = root["parents"][0]["commit"].as_str().unwrap_or("none");
        (ci, parent.trim_start_matches("sha1:").to_owned())
    }
}

/// Decision 67's fence: the step that holds the key starts for a push run of the branch
/// whose commit is on the fetched branch's first-parent line. A pull request's run, a run
/// for another branch, and a push run named like the branch for a pull request's own
/// commit, which a fork's branch or a tag of that name leaves, ring nothing; nor does a
/// name that is not one commit's, since grep reads each line of it as a pattern.
#[test]
fn the_doorbell_rings_only_for_a_push_run_on_the_branch_s_first_parent_line() {
    let f = forge();
    let (head, tip) = f.merged(1);
    let two = format!("deadbeef\n{tip}");
    let rows = [
        ("push", "main", two.as_str(), false),
        ("push", "main", &tip[..7], false),
        ("push", "main", tip.as_str(), true),
        ("push", "main", f.main.as_str(), true),
        ("pull_request", "main", tip.as_str(), false),
        ("push", "pr1", tip.as_str(), false),
        ("push", "main", head.as_str(), false),
        ("push", "main", "", false),
    ];
    for (event, branch, sha, rings) in rows {
        let ran = f.bell(event, branch, sha);
        let want = format!("ring={rings}\n");
        assert_eq!(ran.code, 0, "{event} {branch} {sha}: {}", ran.text);
        assert!(
            ran.text.starts_with(&want),
            "{event} {branch} {sha}: {}",
            ran.text
        );
        assert_eq!(
            ran.text.contains("jtac: nothing judged"),
            !rings,
            "{}",
            ran.text
        );
    }
}

/// Section 15.2's results file as `results.jq` writes it from the forge's listing: the
/// strict reader takes it, a word the forge has not given is left out, a run with no ref
/// matches no branch, and a run a second page repeats is written once.
#[test]
fn the_results_filter_writes_a_file_the_strict_reader_takes() {
    let commit = "0123456789abcdef0123456789abcdef01234567";
    let run = |id: u64, branch: Value, status: &str, conclusion: Value| {
        json!({
            "id": id, "path": WORKFLOW, "head_sha": commit, "event": "push",
            "head_branch": branch, "run_attempt": 2, "status": status, "conclusion": conclusion,
            "name": "ci", "url": "https://example.com/run",
        })
    };
    let done = run(7, "main".into(), "completed", "success".into());
    let runs = [
        done.clone(),
        done,
        run(8, Value::Null, "completed", "failure".into()),
        run(9, "main".into(), "in_progress", Value::Null),
    ];
    let jobs = [
        json!({"id": 70, "run_id": 7, "name": "e2e", "conclusion": "success", "steps": []}),
        json!({"id": 70, "run_id": 7, "name": "e2e", "conclusion": "success", "steps": []}),
        json!({"id": 90, "run_id": 9, "name": "e2e", "conclusion": null}),
    ];
    let dir = Repo::empty();
    let lines = |rows: &[Value]| rows.iter().map(|r| format!("{r}\n")).collect::<String>();
    dir.write("runs.json", lines(&runs).as_bytes());
    dir.write("jobs.json", lines(&jobs).as_bytes());
    let filter = adapter("results.jq");
    let args = [
        "-n",
        "--slurpfile",
        "runs",
        "runs.json",
        "--slurpfile",
        "jobs",
        "jobs.json",
        "-f",
        &filter,
    ];
    let out = dir.command("jq", &args);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let wrote: Value = serde_json::from_slice(&out.stdout).unwrap();
    let want = json!({"v": 1, "forge": "github", "runs": [
        {"workflow": WORKFLOW, "commit": format!("sha1:{commit}"), "event": "push", "ref": "main",
         "id": 7, "attempt": 2, "status": "completed", "conclusion": "success",
         "jobs": [{"name": "e2e", "conclusion": "success"}]},
        {"workflow": WORKFLOW, "commit": format!("sha1:{commit}"), "event": "push", "ref": "",
         "id": 8, "attempt": 2, "status": "completed", "conclusion": "failure", "jobs": []},
        {"workflow": WORKFLOW, "commit": format!("sha1:{commit}"), "event": "push", "ref": "main",
         "id": 9, "attempt": 2, "status": "in_progress", "jobs": [{"name": "e2e"}]},
    ]});
    assert_eq!(wrote, want);
    let results = Results::from_json(&out.stdout).unwrap();
    let pin = Pin {
        workflow: WORKFLOW.into(),
        job: Some("e2e".into()),
    };
    let oid: GitOid = format!("sha1:{commit}").parse().unwrap();
    let read = select(&results, &pin, &oid, &Branch::new("main").unwrap());
    assert!(matches!(read, Selected::Pending), "run 9 is still going");
}

/// Decision 67's order: a merge whose workflow finished first waits behind an older merge
/// the forge still runs, so each root names its parent; the older merge's completion
/// writes both, oldest first. A completion for a tip the gate reads is a re-run's: one
/// that passes keeps the root, and one that fails writes its record beside it (decision 66).
#[test]
fn the_drain_writes_roots_oldest_first_and_waits_behind_a_result_still_running() {
    let f = forge();
    let (_, first) = f.merged(1);
    let (_, second) = f.merged(2);
    f.pushes(&[
        (&first, &second, "pr_merge"),
        (&f.main, &first, "pr_merge"),
        (ZERO, &f.main, "branch_creation"),
    ]);
    f.runs(&first, &[Run::of(None)]);
    f.runs(&second, &[Run::of(Some("success"))]);
    let ran = f.drain(&second);
    assert_eq!(ran.code, 0, "{}", ran.text);
    let waits = format!("jtac: no root yet for {}", &first[..7]);
    assert!(ran.text.contains(&waits), "{}", ran.text);
    assert!(
        !ran.text.contains(&format!("judging {}", &second[..7])),
        "{}",
        ran.text
    );
    assert_eq!(f.reads(&first).0, "none");
    assert_eq!(f.reads(&second).0, "none");

    f.runs(&first, &[Run::of(Some("success"))]);
    let ran = f.drain(&first);
    assert_eq!(ran.code, 0, "{}", ran.text);
    assert_eq!(f.reads(&first), ("covered".into(), f.main.clone()));
    assert_eq!(f.reads(&second), ("covered".into(), first.clone()));

    let ran = f.drain(&second);
    assert_eq!(ran.code, 0, "{}", ran.text);
    assert_eq!(f.reads(&second), ("covered".into(), first.clone()));
    f.runs(&second, &[Run::of(Some("failure"))]);
    let ran = f.drain(&second);
    assert_eq!(ran.code, 1, "{}", ran.text);
    let records = f.origin.git(&["for-each-ref", "refs/jtac/failed"]);
    assert!(
        records.contains(&second) && !records.contains(&first),
        "{records}"
    );
    assert_eq!(f.reads(&second).0, "covered");
}

/// Decision 67's fences inside the job that holds the key. A push the forge lists is
/// judged only when the fetched branch holds its tip on the first-parent line: not a merge
/// a force push took off the branch, and not a pull request's own commit. The tip a force
/// push left is refused, never judged. And the walk ends at a merge whose tree holds no
/// policy, which no gate can judge, so nothing at or below it is judged either.
#[test]
fn the_drain_judges_no_tip_off_the_branch_s_line_or_below_a_merge_with_no_policy() {
    let f = forge();
    let (_, gone) = f.merged(1);
    f.dev
        .git(&["push", "-q", "-f", "origin", &format!("{}:main", f.main)]);
    let (head, second) = f.merged(2);
    f.runs(&gone, &[Run::of(Some("success"))]);
    f.runs(&second, &[Run::of(Some("success"))]);
    f.fixture(&format!("{SLUG}/commits/{head}/pulls"), &json!([]));
    let rewritten = [
        (f.main.as_str(), head.as_str(), "pr_merge"),
        (f.main.as_str(), second.as_str(), "pr_merge"),
        (gone.as_str(), f.main.as_str(), "force_push"),
        (f.main.as_str(), gone.as_str(), "pr_merge"),
    ];
    f.pushes(&rewritten);
    let ran = f.drain(&second);
    assert_eq!(ran.code, 0, "{}", ran.text);
    let judged = |ran: &Ran, commit: &str| ran.text.contains(&format!("judging {}", &commit[..7]));
    assert!(!judged(&ran, &gone) && !judged(&ran, &head), "{}", ran.text);
    assert_eq!(f.reads(&gone).0, "none");
    assert_eq!(f.reads(&second).0, "covered");
    f.runs(&f.main, &[Run::of(Some("success"))]);
    let ran = f.drain(&f.main);
    assert_eq!(ran.code, 2, "{}", ran.text);
    assert!(
        ran.text.contains("this push rewrote the branch"),
        "{}",
        ran.text
    );

    let (_, bare) = f.land(3, false, &|dev| dev.remove(".jtac/policy.toml"));
    let (_, third) = f.land(4, false, &|dev| {
        dev.git(&["checkout", &f.main, "--", ".jtac/policy.toml"]);
    });
    f.land(5, false, &|dev| {
        dev.write("pr5.txt", b"x\n");
    });
    let tip = f.ci.git(&["rev-parse", "HEAD"]);
    f.runs(&tip, &[Run::of(Some("success"))]);
    f.runs(&third, &[Run::of(Some("success"))]);
    f.pushes(&[
        (&third, &tip, "pr_merge"),
        (&bare, &third, "pr_merge"),
        (&second, &bare, "pr_merge"),
        (&f.main, &second, "pr_merge"),
    ]);
    let ran = f.drain(&tip);
    assert!(judged(&ran, &tip) && judged(&ran, &third), "{}", ran.text);
    assert!(
        !judged(&ran, &bare) && !judged(&ran, &second),
        "{}",
        ran.text
    );
}

/// Decision 67's other answers. A merge with no run of the file its policy reads fails the
/// job and holds no later root back, though another file's run lists a job of the check's
/// name. A failed merge is judged again when its own workflow completes, a re-run's
/// result, and not when another's does. A tag of the branch's name stops the job before
/// any results file, and so does a completion for a commit the push list does not hold.
#[test]
fn a_merge_with_no_run_fails_the_job_alone_and_a_re_run_is_read_for_its_own_commit() {
    let f = forge();
    let (_, first) = f.merged(1);
    let (_, second) = f.merged(2);
    f.pushes(&[(&first, &second, "pr_merge"), (&f.main, &first, "pr_merge")]);
    let other = Run {
        workflow: ".github/workflows/other.yml",
        ..Run::of(Some("success"))
    };
    f.runs(&first, &[other]);
    f.runs(&second, &[Run::of(Some("failure"))]);
    let ran = f.drain(&second);
    assert_eq!(ran.code, 2, "{}", ran.text);
    assert!(ran.text.contains("`e2e` is missing"), "{}", ran.text);
    assert_eq!(f.reads(&first).0, "none");
    assert_eq!(f.reads(&second).0, "failed");

    let (_, third) = f.merged(3);
    f.pushes(&[
        (&second, &third, "pr_merge"),
        (&first, &second, "pr_merge"),
        (&f.main, &first, "pr_merge"),
    ]);
    f.runs(&third, &[Run::of(Some("success"))]);
    let ran = f.drain(&third);
    assert_eq!(ran.code, 0, "{}", ran.text);
    assert!(
        !ran.text.contains(&format!("judging {}", &second[..7])),
        "{}",
        ran.text
    );
    assert_eq!(f.reads(&third), ("covered".into(), "none".into()));

    f.runs(&second, &[Run::of(Some("success"))]);
    let ran = f.drain(&second);
    assert_eq!(ran.code, 0, "{}", ran.text);
    assert_eq!(f.reads(&second).0, "covered");

    let ran = f.drain(&f.main);
    assert_eq!(ran.code, 2, "{}", ran.text);
    assert!(
        ran.text.contains("so its old tip is not known"),
        "{}",
        ran.text
    );
    f.dev
        .git(&["push", "-q", "origin", "origin/main:refs/tags/main"]);
    f.runs(&first, &[Run::of(Some("success"))]);
    let ran = f.drain(&first);
    assert_eq!(ran.code, 2, "{}", ran.text);
    assert!(ran.text.contains("holds a tag named main"), "{}", ran.text);
    assert_eq!(f.reads(&first).0, "none");
}
