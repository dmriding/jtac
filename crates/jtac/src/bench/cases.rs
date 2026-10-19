//! The controls `jtac bench` attacks, one function each, in the order section 13 lists them.
//! A case returns the gate's own line and whether the control held; a setup refusal is a row
//! that did not hold, with the refusal as its line.

use jtac_gate::classify::{Action, Bypass, classify};

use super::Row;
use super::scratch::{Lab, Repo};
use crate::refuse::Refusal;

const TEST: &str = "#[test]\nfn keeps() {}\nfn a() {}\n";
const PUSH: &[&str] = &["push", "origin", "main"];

/// Every control, run in order. A lab that cannot be built is a refusal, never a row.
pub fn run(lab: &Lab) -> Result<Vec<Row>, Refusal> {
    let rows: [(&'static str, &'static str, &'static str, Case); 10] = [
        ("clean change", "a change with its root", "passed", clean),
        ("no root", "push without `jtac check`", "refused", no_root),
        ("forged signature", "root.sig replaced", "refused", forged),
        (
            "different tree",
            "root.json naming another tree",
            "refused",
            other_tree,
        ),
        (
            "removed test",
            "a `#[test]` deleted",
            "refused",
            removed_test,
        ),
        (
            "new suppression",
            "an `#[allow]` added",
            "refused",
            suppression,
        ),
        (
            "hook bypass",
            "every spelling in the table",
            "noted",
            bypasses,
        ),
        (
            "unreadable policy",
            "`.jtac/policy.toml` that does not parse",
            "refused",
            bad_policy,
        ),
        (
            "check cannot spawn",
            "a check naming a missing tool",
            "refused",
            no_spawn,
        ),
        (
            "rewind",
            "force-push of a protected branch",
            "refused",
            rewind,
        ),
    ];
    rows.into_iter()
        .map(|(control, attack, expected, case)| {
            let (observed, held) = case(lab)?;
            Ok(Row {
                control,
                attack,
                expected,
                observed,
                held,
            })
        })
        .collect()
}

type Case = fn(&Lab) -> Result<(String, bool), Refusal>;

fn policy(lab: &Lab, check: &str) -> String {
    format!(
        "version = 1\n[checks.self]\nrun = [\"{check}\", \"--version\"]\n[integrity]\n\
         max_new_suppressions = 0\n[fences]\nprotected_branches = [\"main\"]\n\
         force_push_protected = \"deny\"\n",
        check = check.replace('"', ""),
    )
    .replace("EXE", &lab.exe())
}

fn gated<'a>(lab: &'a Lab, lib_rs: &str) -> Result<Repo<'a>, Refusal> {
    lab.repo(&policy(lab, &lab.exe()), lib_rs)
}

/// The first line holding `needle`, trimmed; `None` is the whole text's first line.
fn line(text: &[u8], needle: Option<&str>) -> String {
    let text = String::from_utf8_lossy(text);
    let found = match needle {
        Some(n) => text.lines().find(|l| l.contains(n)),
        None => text.lines().next(),
    };
    found.unwrap_or("(no output)").trim().to_owned()
}

fn check(repo: &Repo<'_>) -> Result<(i32, Vec<u8>, Vec<u8>), Refusal> {
    let out = repo.jtac(&["check"])?;
    Ok((out.status.code().unwrap_or(-1), out.stdout, out.stderr))
}

fn push(repo: &Repo<'_>) -> Result<(bool, Vec<u8>), Refusal> {
    let out = repo.try_git(&["push", "origin", "main"])?;
    Ok((out.status.success(), out.stderr))
}

/// A refused push leaves origin's `main` where it was.
fn refused_push(repo: &Repo<'_>, args: &[&str], needle: &str) -> Result<(String, bool), Refusal> {
    let before = repo.git_in(&repo.bare.clone(), &["rev-parse", "main"], None)?;
    let out = repo.try_git(args)?;
    let (ok, stderr) = (out.status.success(), out.stderr);
    let after = repo.git_in(&repo.bare.clone(), &["rev-parse", "main"], None)?;
    let text = String::from_utf8_lossy(&stderr);
    Ok((
        line(&stderr, Some(needle)),
        !ok && before == after && text.contains(needle),
    ))
}

/// The root reached origin as the same object: the hook's own push, not only the code's.
fn clean(lab: &Lab) -> Result<(String, bool), Refusal> {
    let repo = gated(lab, "fn a() {}\n")?;
    repo.write("src/lib.rs", "fn a() {}\nfn b() {}\n")?;
    let tip = repo.commit("tip")?;
    let (code, stdout, _) = check(&repo)?;
    let (ok, stderr) = push(&repo)?;
    let landed = repo.git_in(&repo.bare.clone(), &["rev-parse", "main"], None)? == tip;
    let root = repo.git(&["rev-parse", &root_ref(&tip)])?;
    let carried = repo.try_git_in(
        &repo.bare.clone(),
        &["rev-parse", "--verify", &root_ref(&tip)],
    )?;
    let first = line(&stdout, None);
    let held = code == 0
        && first.starts_with("CLEARED HOT")
        && ok
        && landed
        && String::from_utf8_lossy(&carried.stdout).trim() == root
        && String::from_utf8_lossy(&stderr).contains("has a root, covered");
    Ok((first, held))
}

fn no_root(lab: &Lab) -> Result<(String, bool), Refusal> {
    let repo = gated(lab, "fn a() {}\n")?;
    repo.write("src/lib.rs", "fn a() {}\nfn b() {}\n")?;
    repo.commit("tip")?;
    refused_push(&repo, PUSH, "has no root")
}

fn root_ref(commit: &str) -> String {
    format!("refs/jtac/roots/{}/{commit}", &commit[..2])
}

fn forged(lab: &Lab) -> Result<(String, bool), Refusal> {
    let repo = gated(lab, "fn a() {}\n")?;
    repo.write("src/lib.rs", "fn a() {}\nfn b() {}\n")?;
    let tip = repo.commit("tip")?;
    let (code, stdout, _) = check(&repo)?;
    if code != 0 {
        return Ok((line(&stdout, None), false));
    }
    let name = root_ref(&tip);
    let json = repo.git(&["rev-parse", &format!("{name}:root.json")])?;
    let sig = repo.git_in(
        &repo.path.clone(),
        &["hash-object", "-w", "--stdin"],
        Some("not a signature\n"),
    )?;
    let listing = format!("100644 blob {json}\troot.json\n100644 blob {sig}\troot.sig\n");
    let tree = repo.git_in(&repo.path.clone(), &["mktree"], Some(&listing))?;
    let commit = repo.git(&["commit-tree", &tree, "-m", "forged"])?;
    repo.git(&["update-ref", &name, &commit])?;
    refused_push(&repo, PUSH, "its signature does not hold")
}

/// The root keeps its commit and signature; only the tree it names is changed.
fn other_tree(lab: &Lab) -> Result<(String, bool), Refusal> {
    let repo = gated(lab, "fn a() {}\n")?;
    repo.write("src/lib.rs", "fn a() {}\nfn b() {}\n")?;
    let tip = repo.commit("tip")?;
    let (code, stdout, _) = check(&repo)?;
    if code != 0 {
        return Ok((line(&stdout, None), false));
    }
    let name = root_ref(&tip);
    let own = repo.git(&["rev-parse", "HEAD^{tree}"])?;
    let other = repo.git(&["rev-parse", "HEAD~1^{tree}"])?;
    let edited = repo
        .git(&["cat-file", "-p", &format!("{name}:root.json")])?
        .replace(&own, &other);
    let json = repo.git_in(
        &repo.path.clone(),
        &["hash-object", "-w", "--stdin"],
        Some(&edited),
    )?;
    let sig = repo.git(&["rev-parse", &format!("{name}:root.sig")])?;
    let listing = format!("100644 blob {json}\troot.json\n100644 blob {sig}\troot.sig\n");
    let tree = repo.git_in(&repo.path.clone(), &["mktree"], Some(&listing))?;
    let commit = repo.git(&["commit-tree", &tree, "-m", "edited"])?;
    repo.git(&["update-ref", &name, &commit])?;
    refused_push(&repo, PUSH, "names tree")
}

fn aborted(repo: &Repo<'_>, rule: &str) -> Result<(String, bool), Refusal> {
    let (code, stdout, _) = check(repo)?;
    let text = String::from_utf8_lossy(&stdout);
    Ok((
        line(&stdout, Some(rule)),
        code == 1 && text.starts_with("ABORT") && text.contains(rule),
    ))
}

fn removed_test(lab: &Lab) -> Result<(String, bool), Refusal> {
    let repo = gated(lab, TEST)?;
    repo.write("src/lib.rs", "fn a() {}\n")?;
    repo.commit("tip")?;
    aborted(&repo, "JT-TEST-001")
}

fn suppression(lab: &Lab) -> Result<(String, bool), Refusal> {
    let repo = gated(lab, "fn a() {}\n")?;
    repo.write("src/lib.rs", "#[allow(dead_code)]\nfn a() {}\n")?;
    repo.commit("tip")?;
    aborted(&repo, "JT-SUP-001")
}

/// Every spelling the table marks as a bypass classifies as one, and the advisory names what
/// lands wherever one is due: on a push, and on a bypass that outlives its command. A bypass
/// on a commit, merge or am alone is silent by design (section 11.3). Pure, so no repository.
fn bypasses(_: &Lab) -> Result<(String, bool), Refusal> {
    let scripts = jtac_gate::bypasses();
    let (mut named, mut noted, mut silent) = (0, 0, 0);
    for script in &scripts {
        let actions = classify(script);
        let due = actions.iter().any(|a| match a {
            Action::Push(p) => p.bypass.is_some(),
            Action::HookBypass { spelling, .. } => matches!(
                spelling,
                Bypass::ConfigWrite | Bypass::ConfigParameters | Bypass::ConfigEnvVar
            ),
            Action::ChangeDir { .. } | Action::Unreadable { .. } => false,
        });
        let is_bypass = due
            || actions
                .iter()
                .any(|a| matches!(a, Action::HookBypass { .. }));
        let note = jtac_gate::note(&actions, &|_| None);
        let right = match (due, &note) {
            (true, Some(n)) => n.contains("no root") || n.contains("uncovered"),
            (false, None) => true,
            _ => false,
        };
        if is_bypass && right {
            named += 1;
            if due {
                noted += 1;
            } else {
                silent += 1;
            }
        }
    }
    let example = jtac_gate::note(&classify("git push --no-verify"), &|_| None).unwrap_or_default();
    Ok((
        format!(
            "{named} of {} spellings named, {noted} with an advisory, {silent} silent by design; {example}",
            scripts.len()
        ),
        named == scripts.len() && !scripts.is_empty() && noted > 0,
    ))
}

fn bad_policy(lab: &Lab) -> Result<(String, bool), Refusal> {
    let repo = gated(lab, "fn a() {}\n")?;
    repo.write(".jtac/policy.toml", "version = \n")?;
    repo.commit("tip")?;
    let (code, _, stderr) = check(&repo)?;
    let text = String::from_utf8_lossy(&stderr);
    Ok((
        line(&stderr, None),
        code == 2 && text.contains(".jtac/policy.toml") && text.contains("negotiation"),
    ))
}

fn no_spawn(lab: &Lab) -> Result<(String, bool), Refusal> {
    let missing = format!(
        "{}/no-such-tool",
        lab.exe().rsplit_once('/').map_or("", |(d, _)| d)
    );
    let repo = lab.repo(&policy(lab, &missing), "fn a() {}\n")?;
    repo.write("src/lib.rs", "fn a() {}\nfn b() {}\n")?;
    repo.commit("tip")?;
    aborted(&repo, "check `self` error")
}

fn rewind(lab: &Lab) -> Result<(String, bool), Refusal> {
    let repo = gated(lab, "fn a() {}\n")?;
    repo.write("src/lib.rs", "fn a() {}\nfn b() {}\n")?;
    repo.commit("published")?;
    let (code, stdout, _) = check(&repo)?;
    let (ok, _) = push(&repo)?;
    if code != 0 || !ok {
        return Ok((line(&stdout, None), false));
    }
    repo.git(&["reset", "-q", "--hard", "HEAD~1"])?;
    repo.write("src/lib.rs", "fn a() {}\nfn c() {}\n")?;
    repo.commit("rewritten")?;
    let (code, stdout, _) = check(&repo)?;
    if code != 0 {
        return Ok((line(&stdout, None), false));
    }
    refused_push(
        &repo,
        &["push", "--force", "origin", "main"],
        "rewinds a protected branch",
    )
}
