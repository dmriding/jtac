//! `jtac hook pre-tool claude-code` against a throwaway clone (DESIGN.md section 11.3): the
//! agent's JSON on stdin, the note on stdout as `additionalContext`, exit 0 on every path,
//! and the same answer `status` and the `pre-push` hook give.

use std::io::Write as _;
use std::process::{Command, Stdio};

use jtac_testkit::Repo;
use serde_json::Value;

fn me() -> String {
    env!("CARGO_BIN_EXE_jtac").replace('\\', "/")
}

/// The binary under coverage drops a profile file where it runs; ignored, it leaves the tree
/// clean for the next check.
fn with_policy(repo: &Repo) {
    let policy = format!(
        "version = 1\n[checks.ok]\nrun = [\"{}\", \"--version\"]\n",
        me()
    );
    repo.write(".jtac/policy.toml", policy.as_bytes());
    repo.write(".gitignore", b"*.profraw\n");
}

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

struct Ran {
    code: i32,
    stdout: String,
    stderr: String,
}

/// The hook as the agent runs it: its JSON on stdin, the repository as cwd unless `cwd`
/// names another, `args` after `pre-tool`.
fn pre_tool(repo: &Repo, args: &[&str], stdin: &[u8]) -> Ran {
    let mut child = Command::new(env!("CARGO_BIN_EXE_jtac"))
        .args([&["hook", "pre-tool"], args].concat())
        .current_dir(repo.path())
        .env_clear()
        .envs(repo.env())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(stdin).unwrap();
    let out = child.wait_with_output().unwrap();
    Ran {
        code: out.status.code().unwrap_or(-1),
        stdout: String::from_utf8(out.stdout).unwrap(),
        stderr: String::from_utf8(out.stderr).unwrap(),
    }
}

fn bash(cwd: Option<&str>, command: &str) -> Vec<u8> {
    let mut v = serde_json::json!({
        "session_id": "s", "hook_event_name": "PreToolUse", "tool_name": "Bash",
        "tool_input": {"command": command, "description": "d"},
    });
    if let Some(cwd) = cwd {
        v["cwd"] = Value::String(cwd.to_owned());
    }
    v.to_string().into_bytes()
}

/// The note a run returned, `None` when stdout was empty; exit 0 either way.
fn note(ran: &Ran) -> Option<String> {
    assert_eq!(ran.code, 0, "{}{}", ran.stdout, ran.stderr);
    if ran.stdout.is_empty() {
        return None;
    }
    let v: Value = serde_json::from_str(&ran.stdout).unwrap();
    let out = &v["hookSpecificOutput"];
    assert_eq!(out["hookEventName"], "PreToolUse", "{v}");
    assert!(out.get("permissionDecision").is_none(), "{v}");
    Some(out["additionalContext"].as_str().unwrap().to_owned())
}

/// `base` on origin's `main` under the default fences, a key, `covered` on top with a root.
struct Checked {
    repo: Repo,
    base: String,
}

fn checked() -> Checked {
    let origin = Repo::bare(&[]);
    let repo = Repo::clone_of(&origin);
    with_policy(&repo);
    repo.write("src/lib.rs", b"fn a() {}\n");
    let base = repo.commit("base");
    repo.git(&["push", "-q", "-u", "origin", "main"]);
    keyed(&repo);
    repo.write("a.txt", b"a\n");
    repo.commit("covered");
    let out = repo.command(env!("CARGO_BIN_EXE_jtac"), &["check"]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    std::mem::forget(origin);
    Checked { repo, base }
}

#[test]
fn a_push_of_an_uncovered_tip_names_jtac_check_and_a_covered_one_is_silent() {
    let Checked { repo, .. } = checked();
    let cwd = repo.path().to_string_lossy().replace('\\', "/");
    for script in [
        "git push",
        "git push origin main",
        "git push origin HEAD:main",
    ] {
        let ran = pre_tool(&repo, &["claude-code"], &bash(Some(&cwd), script));
        assert_eq!(note(&ran), None, "{script}: {}", ran.stderr);
    }
    repo.write("b.txt", b"b\n");
    repo.commit("bare");
    for script in [
        "git push",
        "git push origin main",
        "cd . && git push -u origin main",
    ] {
        let ran = pre_tool(&repo, &["claude-code"], &bash(Some(&cwd), script));
        let n = note(&ran).unwrap_or_default();
        assert!(
            n.contains("no root") && n.contains("Run `jtac check` first"),
            "{script}: {n}"
        );
    }
    let ran = pre_tool(&repo, &["claude-code"], &bash(None, "git push --dry-run"));
    assert_eq!(note(&ran), None);
    let other = pre_tool(
        &repo,
        &["claude-code"],
        &bash(Some(&cwd), "git push origin HEAD:topic"),
    );
    let n = note(&other).unwrap_or_default();
    assert!(
        n.contains("no root"),
        "a new branch still needs a root: {n}"
    );
}

#[test]
fn a_rewind_of_a_protected_branch_is_the_fence_and_a_bypass_says_what_lands() {
    let Checked { repo, base } = checked();
    repo.git(&["push", "-q", "origin", "main"]);
    repo.git(&["reset", "-q", "--hard", &base]);
    repo.write("c.txt", b"c\n");
    repo.commit("rework");
    let out = repo.command(env!("CARGO_BIN_EXE_jtac"), &["check"]);
    assert!(out.status.success());
    let ran = pre_tool(
        &repo,
        &["claude-code"],
        &bash(None, "git push --force origin main"),
    );
    let n = note(&ran).unwrap();
    assert!(
        n.contains("rewinds a protected branch") && n.contains("no clearing command"),
        "{n}"
    );
    let ran = pre_tool(
        &repo,
        &["claude-code"],
        &bash(None, "git push --no-verify origin main"),
    );
    let n = note(&ran).unwrap();
    assert!(
        n.contains("skips the pre-push hook") && n.contains("`jtac verify`"),
        "{n}"
    );
    assert!(!n.contains("Run `"), "a bypass names no command: {n}");
    let ran = pre_tool(
        &repo,
        &["claude-code"],
        &bash(None, "git push --delete origin main"),
    );
    let n = note(&ran).unwrap();
    assert!(n.contains("deletes a protected branch"), "{n}");
}

#[test]
fn an_unnamed_directory_a_non_repository_and_an_unreadable_script_are_short_notes() {
    let Checked { repo, .. } = checked();
    repo.write("b.txt", b"b\n");
    repo.commit("bare");
    let ran = pre_tool(
        &repo,
        &["claude-code"],
        &bash(None, "cd \"$D\" && git push"),
    );
    let n = note(&ran).unwrap();
    assert!(
        n.contains("could not be judged") && n.contains("`jtac status`"),
        "{n}"
    );
    let elsewhere = Repo::empty();
    let far = elsewhere.path().to_string_lossy().replace('\\', "/");
    let ran = pre_tool(&repo, &["claude-code"], &bash(Some(&far), "git push"));
    assert!(
        note(&ran).unwrap().contains("could not be judged"),
        "{}",
        ran.stdout
    );
    let ran = pre_tool(
        &repo,
        &["claude-code"],
        &bash(None, "eval \"git push $ARGS\""),
    );
    assert!(
        note(&ran).unwrap().contains("nothing in it was judged"),
        "{}",
        ran.stdout
    );
    let ran = pre_tool(
        &repo,
        &["claude-code"],
        &bash(None, "cargo test && bash -c \"$X\""),
    );
    assert_eq!(note(&ran), None, "{}", ran.stdout);
}

#[test]
fn every_failure_prints_nothing_and_exits_zero_with_the_reason_on_stderr() {
    let repo = Repo::init(&[]);
    let rows: [(&str, &[u8], &str); 4] = [
        (
            "another tool",
            br#"{"tool_name":"Edit","tool_input":{"file_path":"x"}}"#,
            "",
        ),
        ("not json", b"git push", "stdin"),
        ("not utf-8", b"\xff{}", "UTF-8"),
        (
            "a command that is not a string",
            br#"{"tool_name":"Bash","tool_input":{"command":1}}"#,
            "not a string",
        ),
    ];
    for (claim, stdin, stderr) in rows {
        let ran = pre_tool(&repo, &["claude-code"], stdin);
        assert_eq!(ran.code, 0, "{claim}: {}", ran.stderr);
        assert_eq!(ran.stdout, "", "{claim}");
        assert!(ran.stderr.contains(stderr), "{claim}: {:?}", ran.stderr);
    }
    let ran = pre_tool(&repo, &["claude-code"], &bash(None, "git push"));
    assert!(
        note(&ran).unwrap().contains("could not be judged"),
        "no commit yet: {}",
        ran.stdout
    );
    assert!(
        ran.stderr.contains("git push"),
        "the reason: {}",
        ran.stderr
    );
    let ran = pre_tool(&repo, &["other-agent"], b"{}");
    assert_eq!((ran.code, ran.stdout.as_str()), (0, ""));
    assert!(
        ran.stderr.contains("no adapter for `other-agent`") && ran.stderr.contains("claude-code"),
        "{}",
        ran.stderr
    );
}

/// Refspec forms resolve as git resolves them: a `:dst` beside an update deletes only its
/// own ref; `HEAD` is the current branch, not the one it merges with, while a bare push is;
/// a tag by bare name or full ref passes on its name; a wildcard and a word the shell built
/// cannot be placed; the old tip follows the remote's fetch refspec.
#[test]
fn refspecs_resolve_as_git_does_and_the_old_tip_follows_the_fetch_refspec() {
    let Checked { repo, base } = checked();
    let ran = pre_tool(
        &repo,
        &["claude-code"],
        &bash(None, "git push origin main :topic"),
    );
    assert_eq!(note(&ran), None, "{}", ran.stdout);
    repo.git(&["tag", "v1"]);
    repo.write("b.txt", b"b\n");
    repo.commit("bare");
    repo.git(&["tag", "v2"]);
    for script in [
        "git push origin v1",
        "git push origin v2",
        "git push origin refs/tags/v2",
        "git push origin tag v2",
        "git push origin --tags",
    ] {
        let ran = pre_tool(&repo, &["claude-code"], &bash(None, script));
        assert_eq!(note(&ran), None, "{script}: {}", ran.stdout);
    }
    for script in [
        "git push origin 'refs/heads/rel/*:refs/heads/rel/*'",
        "git push origin \"$BRANCH\"",
        "git push --all origin",
    ] {
        let ran = pre_tool(&repo, &["claude-code"], &bash(None, script));
        let n = note(&ran).unwrap_or_default();
        assert!(n.contains("could not be judged"), "{script}: {n}");
    }
    repo.git(&["push", "-q", "origin", "main"]);
    repo.git(&["switch", "-q", "-c", "feature", &base]);
    repo.git(&["branch", "--set-upstream-to=origin/main"]);
    repo.write("c.txt", b"c\n");
    repo.commit("rework");
    let out = repo.command(env!("CARGO_BIN_EXE_jtac"), &["check"]);
    assert!(out.status.success());
    let ran = pre_tool(&repo, &["claude-code"], &bash(None, "git push origin HEAD"));
    assert_eq!(note(&ran), None, "HEAD is feature: {}", ran.stdout);
    let ran = pre_tool(&repo, &["claude-code"], &bash(None, "git push"));
    let n = note(&ran).unwrap();
    assert!(
        n.contains("rewinds a protected branch"),
        "bare push merges with main: {n}"
    );
    repo.git(&[
        "config",
        "remote.origin.fetch",
        "+refs/heads/*:refs/remotes/company/*",
    ]);
    let main = repo.git(&["rev-parse", "refs/remotes/origin/main"]);
    repo.git(&["update-ref", "refs/remotes/company/main", &main]);
    repo.git(&["update-ref", "-d", "refs/remotes/origin/main"]);
    let ran = pre_tool(
        &repo,
        &["claude-code"],
        &bash(None, "git push origin HEAD:main"),
    );
    let n = note(&ran).unwrap();
    assert!(
        n.contains("rewinds a protected branch"),
        "through the fetch refspec: {n}"
    );
    let url = repo.git(&["config", "remote.origin.url"]);
    for script in [
        format!("git push -f {url} HEAD:main"),
        "git push -f origin HEAD:heads/main".to_owned(),
    ] {
        let ran = pre_tool(&repo, &["claude-code"], &bash(None, &script));
        let n = note(&ran).unwrap_or_default();
        assert!(n.contains("rewinds a protected branch"), "{script}: {n}");
    }
}

/// A bare destination is the remote's ref of that name before the source's namespace, as git
/// resolves it; `@` is HEAD; of several refspecs the fence is the note; a remote given by
/// URL with a trailing slash or through `insteadOf` still finds the old tip.
#[test]
fn a_bare_destination_is_the_remote_ref_of_that_name_and_a_fence_outranks_a_missing_root() {
    let Checked { repo, base } = checked();
    repo.git(&["push", "-q", "origin", "main"]);
    repo.git(&["tag", "v1", &base]);
    repo.git(&["push", "-q", "origin", "v1"]);
    let ran = pre_tool(
        &repo,
        &["claude-code"],
        &bash(None, "git push --force origin v1:main"),
    );
    let n = note(&ran).unwrap_or_default();
    assert!(
        n.contains("rewinds a protected branch"),
        "a tag to the branch main: {n}"
    );
    repo.git(&["switch", "-q", "-c", "wip"]);
    repo.write("w.txt", b"w\n");
    repo.commit("wip");
    let ran = pre_tool(
        &repo,
        &["claude-code"],
        &bash(None, "git push --force origin wip:v1"),
    );
    let n = note(&ran).unwrap_or_default();
    assert!(
        n.contains("could not be judged"),
        "a branch onto a tag's name: the remote's tags are not held here: {n}"
    );
    repo.git(&["branch", "-f", "main", &base]);
    let ran = pre_tool(
        &repo,
        &["claude-code"],
        &bash(None, "git push --force origin wip:wip main"),
    );
    let n = note(&ran).unwrap_or_default();
    assert!(
        n.contains("rewinds a protected branch"),
        "the fence, not wip's missing root: {n}"
    );
    repo.git(&["switch", "-q", "main"]);
    let url = repo.git(&["config", "remote.origin.url"]);
    let instead = format!("url.{url}.insteadOf");
    repo.git(&["config", &instead, "example-app"]);
    for script in [
        "git push --force origin @".to_owned(),
        format!("git push -f {url}/ main"),
        "git push -f example-app main".to_owned(),
    ] {
        let ran = pre_tool(&repo, &["claude-code"], &bash(None, &script));
        let n = note(&ran).unwrap_or_default();
        assert!(n.contains("rewinds a protected branch"), "{script}: {n}");
    }
    repo.git(&["push", "-q", "origin", "main:rel2"]);
    repo.git(&["tag", "rel2", "wip"]);
    let ran = pre_tool(&repo, &["claude-code"], &bash(None, "git push origin rel2"));
    assert_eq!(note(&ran), None, "no colon: the local tag: {}", ran.stdout);
    let ran = pre_tool(
        &repo,
        &["claude-code"],
        &bash(None, "git push origin rel2:rel2"),
    );
    let n = note(&ran).unwrap_or_default();
    assert!(n.contains("no root"), "spelled: the remote's branch: {n}");
    repo.git(&["tag", "local-only", "wip"]);
    let ran = pre_tool(
        &repo,
        &["claude-code"],
        &bash(None, "git push origin wip:local-only"),
    );
    let n = note(&ran).unwrap_or_default();
    assert!(n.contains("could not be judged"), "a tag only here: {n}");
    repo.git(&["remote", "add", "never-fetched", &url]);
    repo.git(&["config", "remote.origin.pushurl", "pushalias:origin.git"]);
    for script in [
        format!("git push --force {url} HEAD:main"),
        "git push --force pushalias:origin.git HEAD:main".to_owned(),
    ] {
        let ran = pre_tool(&repo, &["claude-code"], &bash(None, &script));
        let n = note(&ran).unwrap_or_default();
        assert!(n.contains("rewinds a protected branch"), "{script}: {n}");
    }
}
