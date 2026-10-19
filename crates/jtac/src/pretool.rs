//! `jtac hook pre-tool <agent>` (DESIGN.md section 11.3): the agent's proposed command read
//! into actions, each push judged where it would run by the decision the `pre-push` hook
//! asks, and the note handed back on the agent's wire. It advises and never blocks: every
//! failure prints nothing and exits 0, and the work, stdin included, runs on a thread the
//! main one abandons at the cap, so a slow script or a held pipe costs the agent one second.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::Duration;

use jtac_gate::classify::{Dir, Push, classify};
use jtac_gate::{Decision, Pass};
use jtac_store::Store;

use crate::wire::claude;
use crate::{coverage, repo, update};

/// The wall clock the hook gives itself (decided 2026-10-19).
pub const CAP: Duration = Duration::from_millis(1000);

/// What reads the agent's JSON, on the worker.
pub type Stdin = Box<dyn FnOnce() -> Result<String, String> + Send>;

/// Stdout and stderr; the exit code is always 0.
pub fn pre_tool(agent: &str, cwd: &Path, stdin: Stdin) -> (String, String) {
    if agent != claude::NAME {
        let why = format!(
            "jtac: no adapter for `{agent}`; the one built is `{}`\n",
            claude::NAME
        );
        return (String::new(), why);
    }
    let cwd = cwd.to_path_buf();
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let answer = match stdin() {
            Ok(text) => advise(&text, &cwd),
            Err(why) => (None, vec![format!("stdin: {why}")]),
        };
        let _ = tx.send(answer);
    });
    match rx.recv_timeout(CAP) {
        Ok((note, diagnostics)) => (
            note.as_deref().map(claude::output).unwrap_or_default(),
            diagnostics
                .iter()
                .map(|d| format!("jtac: pre-tool: {d}\n"))
                .collect(),
        ),
        Err(_) => (
            String::new(),
            format!(
                "jtac: pre-tool gave up after {}ms; the pre-push hook still judges the push\n",
                CAP.as_millis()
            ),
        ),
    }
}

/// The note and what could not be judged, for stderr.
fn advise(stdin: &str, cwd: &Path) -> (Option<String>, Vec<String>) {
    let call = match claude::call(stdin) {
        Ok(Some(call)) => call,
        Ok(None) => return (None, Vec::new()),
        Err(why) => return (None, vec![why]),
    };
    let here = call.cwd.map_or_else(|| cwd.to_path_buf(), PathBuf::from);
    let actions = classify(&call.command);
    let diagnostics = RefCell::new(Vec::new());
    let note = jtac_gate::note(&actions, &|p| match judge(&here, p) {
        Ok(decision) => decision,
        Err(why) => {
            diagnostics.borrow_mut().push(why);
            None
        }
    });
    (note, diagnostics.into_inner())
}

/// One push's decision, or `None` when where it runs is not named, is not a repository, or
/// its targets cannot be told. The first refused refspec is the answer; a tag or evidence
/// ref passes on its name before any fact is read, as the hook judges it.
fn judge(here: &Path, push: &Push) -> Result<Option<Decision>, String> {
    let dir = match &push.dir {
        Dir::Here => here.to_path_buf(),
        Dir::At(p) => here.join(p),
        Dir::Unknown => return Ok(None),
    };
    let Ok(repo) = repo::root(&dir) else {
        return Ok(None);
    };
    let fail = |e: &dyn std::fmt::Display| format!("{}: {e}", push.raw);
    let Some(targets) = targets(&repo, push).map_err(|e| fail(&e))? else {
        return Ok(None);
    };
    let format = repo::format(&repo).map_err(|e| fail(&e))?;
    let store = Store::open(&repo).map_err(|e| fail(&e))?;
    let tool = coverage::verifier(&repo).map_err(|r| fail(&r.what))?;
    let remote = match &push.remote {
        Some(r) => named(&repo, r).map_err(|e| fail(&e))?,
        None => remote(&repo).map_err(|e| fail(&e))?,
    };
    let key = format!("remote.{remote}.fetch");
    let fetch = repo::probe(&repo, &["config", "--get-all", &key], 1)
        .map_err(|e| fail(&e))?
        .unwrap_or_default();
    let mut last = push.tags.then_some(Decision::Pass(Pass::Tag));
    for (src, dst) in targets {
        if let Some(pass) = jtac_gate::exempt(&dst, src.is_none()) {
            last = Some(Decision::Pass(pass));
            continue;
        }
        let tip = match src {
            Some(rev) => Some(repo::peel(&repo, &format, &rev).map_err(|e| fail(&e))?),
            None => None,
        };
        let old = tracking(&fetch, &remote, &dst)
            .and_then(|r| repo::peel(&repo, &format, &r).ok())
            .map(|h| h.commit);
        let g = update::gather(
            &repo,
            &store,
            &tool,
            &format,
            tip.as_ref(),
            &dst,
            old.as_ref(),
        )
        .map_err(|r| fail(&r.what))?;
        let decision = jtac_gate::decide(&g.update);
        if matches!(decision, Decision::Refuse(_)) {
            return Ok(Some(decision));
        }
        last = Some(decision);
    }
    Ok(last)
}

/// `(source, full remote ref)` per refspec; `None` when the targets cannot be told: every
/// ref, a wildcard, a word the shell built, or a bare push from a detached HEAD. No refspec
/// is HEAD to the branch it merges with, as `status` judges a bare `git push`, or only the
/// tags under `--tags`; a bare name takes its namespace from the source, as git does (`v1`
/// from a tag is `refs/tags/v1`).
/// A source rev (none for a deletion) and the full remote ref it goes to.
type Target = (Option<String>, String);

fn targets(repo: &Path, push: &Push) -> Result<Option<Vec<Target>>, String> {
    if push.all_refs || push.opaque {
        return Ok(None);
    }
    if push.destinations.is_empty() && push.tags {
        return Ok(Some(Vec::new()));
    }
    if push.destinations.is_empty() {
        return Ok(merge_ref(repo)?.map(|dst| vec![(Some("HEAD".to_owned()), dst)]));
    }
    let mut out = Vec::new();
    for (i, dst) in push.destinations.iter().enumerate() {
        let src = push.sources.get(i).filter(|s| !s.is_empty()).cloned();
        let full = if dst.starts_with("refs/") {
            dst.clone()
        } else if let Some(tag) = dst.strip_prefix("tag ") {
            format!("refs/tags/{tag}")
        } else {
            let named = match &src {
                Some(rev) => full_name(repo, rev)?,
                None => None,
            };
            match (dst.as_str(), named) {
                ("HEAD", Some(full)) => full,
                ("HEAD", None) => return Ok(None),
                (d, _)
                    if ["heads/", "tags/", "remotes/"]
                        .iter()
                        .any(|p| d.starts_with(p)) =>
                {
                    format!("refs/{d}")
                }
                (_, Some(full)) if full.starts_with("refs/tags/") => format!("refs/tags/{dst}"),
                _ => format!("refs/heads/{dst}"),
            }
        };
        out.push((src, full));
    }
    Ok(Some(out))
}

/// The remote-tracking ref `remote` keeps for `dst`, through its fetch refspecs; the
/// conventional one when none maps it.
fn tracking(fetch: &str, remote: &str, dst: &str) -> Option<String> {
    for spec in fetch.lines().map(|l| l.trim_start_matches('+')) {
        let Some((src, local)) = spec.split_once(':') else {
            continue;
        };
        match (src.split_once('*'), local.split_once('*')) {
            (Some((head, tail)), Some((lhead, ltail))) => {
                if let Some(mid) = dst.strip_prefix(head).and_then(|r| r.strip_suffix(tail)) {
                    return Some(format!("{lhead}{mid}{ltail}"));
                }
            }
            (None, None) if src == dst => return Some(local.to_owned()),
            _ => {}
        }
    }
    let branch = dst.strip_prefix("refs/heads/")?;
    Some(format!("refs/remotes/{remote}/{branch}"))
}

/// The full ref `rev` names, `None` for a hash or a detached HEAD.
fn full_name(repo: &Path, rev: &str) -> Result<Option<String>, String> {
    let args = ["rev-parse", "--symbolic-full-name", rev];
    let out = repo::probe(repo, &args, 1).map_err(|e| e.to_string())?;
    Ok(out.filter(|f| f.starts_with("refs/")))
}

/// `given` as a remote's name: itself when configured, else the remote whose URL it is, so
/// a push by URL still finds the old tip; else itself, which no fetch refspec maps.
fn named(repo: &Path, given: &str) -> Result<String, String> {
    let url = |r: &str| repo::config(repo, &format!("remote.{r}.url")).map_err(|e| e.to_string());
    if url(given)?.is_some() {
        return Ok(given.to_owned());
    }
    let listed = repo::probe(repo, &["remote"], 1).map_err(|e| e.to_string())?;
    for name in listed.unwrap_or_default().lines() {
        if url(name)?.is_some_and(|u| u == given) {
            return Ok(name.to_owned());
        }
    }
    Ok(given.to_owned())
}

/// The remote a bare `git push` goes to.
fn remote(repo: &Path) -> Result<String, String> {
    let branch = short_branch(repo)?;
    for key in [
        branch.as_ref().map(|b| format!("branch.{b}.pushRemote")),
        Some("remote.pushDefault".to_owned()),
        branch.as_ref().map(|b| format!("branch.{b}.remote")),
    ]
    .into_iter()
    .flatten()
    {
        if let Some(name) = repo::config(repo, &key).map_err(|e| e.to_string())? {
            return Ok(name);
        }
    }
    Ok("origin".to_owned())
}

/// The ref HEAD's branch merges with, else the branch itself; `None` detached.
fn merge_ref(repo: &Path) -> Result<Option<String>, String> {
    let Some(branch) = short_branch(repo)? else {
        return Ok(None);
    };
    let key = format!("branch.{branch}.merge");
    let merge = repo::config(repo, &key).map_err(|e| e.to_string())?;
    Ok(Some(
        merge.unwrap_or_else(|| format!("refs/heads/{branch}")),
    ))
}

fn short_branch(repo: &Path) -> Result<Option<String>, String> {
    let full =
        repo::probe(repo, &["symbolic-ref", "--quiet", "HEAD"], 1).map_err(|e| e.to_string())?;
    Ok(full.and_then(|f| f.strip_prefix("refs/heads/").map(str::to_owned)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_old_tip_follows_the_fetch_refspec_and_falls_back_to_the_conventional_ref() {
        let company = "+refs/heads/*:refs/remotes/company/*\n";
        assert_eq!(
            tracking(company, "origin", "refs/heads/main").as_deref(),
            Some("refs/remotes/company/main")
        );
        let one = "+refs/heads/main:refs/remotes/origin/trunk\n";
        assert_eq!(
            tracking(one, "origin", "refs/heads/main").as_deref(),
            Some("refs/remotes/origin/trunk")
        );
        assert_eq!(
            tracking(one, "origin", "refs/heads/topic").as_deref(),
            Some("refs/remotes/origin/topic")
        );
        assert_eq!(
            tracking("", "up", "refs/heads/x").as_deref(),
            Some("refs/remotes/up/x")
        );
        assert_eq!(tracking("", "up", "refs/tags/v1"), None);
    }
}
