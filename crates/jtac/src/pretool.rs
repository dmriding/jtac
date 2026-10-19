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
use jtac_gate::{Decision, Pass, Refuse};
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
/// its targets cannot be told. A tag or evidence ref passes on its name before any fact is
/// read, as the hook judges it; of several refspecs the worst answer is the note, so a fence
/// never hides behind a missing root.
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
    let remote = match &push.remote {
        Some(r) => named(&repo, r).map_err(|e| fail(&e))?,
        None => remote(&repo).map_err(|e| fail(&e))?,
    };
    let key = format!("remote.{remote}.fetch");
    let fetch = repo::probe(&repo, &["config", "--get-all", &key], 1)
        .map_err(|e| fail(&e))?
        .unwrap_or_default();
    let Some(targets) = targets(&repo, push, &fetch, &remote).map_err(|e| fail(&e))? else {
        return Ok(None);
    };
    let format = repo::format(&repo).map_err(|e| fail(&e))?;
    let store = Store::open(&repo).map_err(|e| fail(&e))?;
    let tool = coverage::verifier(&repo).map_err(|r| fail(&r.what))?;
    let mut last = push.tags.then_some(Decision::Pass(Pass::Tag));
    for (src, dst) in targets {
        let decision = if let Some(pass) = jtac_gate::exempt(&dst, src.is_none()) {
            Decision::Pass(pass)
        } else {
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
            jtac_gate::decide(&g.update)
        };
        if last.as_ref().is_none_or(|l| rank(l) < rank(&decision)) {
            last = Some(decision);
        }
    }
    Ok(last)
}

/// A fence, above a refusal a command clears, above a pass.
fn rank(d: &Decision) -> u8 {
    match d {
        Decision::Pass(_) => 0,
        Decision::Refuse(Refuse::Absent | Refuse::Unknown) => 1,
        Decision::Refuse(Refuse::Rewind | Refuse::Delete) => 2,
    }
}

/// A source rev (none for a deletion) and the full remote ref it goes to.
type Target = (Option<String>, String);

/// One [`Target`] per refspec; `None` when the targets cannot be told: every ref, a
/// wildcard, a word the shell built, a bare push from a detached HEAD, or a bare name whose
/// place on the remote this clone cannot tell. No refspec is HEAD to the branch it merges
/// with, as `status` judges a bare `git push`, or only the tags under `--tags`. A name with
/// no colon is the source's own ref; a spelled bare name is resolved by [`known`].
fn targets(
    repo: &Path,
    push: &Push,
    fetch: &str,
    remote: &str,
) -> Result<Option<Vec<Target>>, String> {
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
        let spelled = push.spelled.get(i).copied().unwrap_or(true);
        let named = match &src {
            Some(rev) => full_name(repo, rev)?,
            None => None,
        };
        let full = if dst.starts_with("refs/") {
            dst.clone()
        } else if !spelled {
            match named {
                Some(full) => full,
                None => return Ok(None),
            }
        } else {
            match (dst.as_str(), named) {
                ("HEAD" | "@", Some(full)) => full,
                ("HEAD" | "@", None) => return Ok(None),
                (d, _)
                    if ["heads/", "tags/", "remotes/"]
                        .iter()
                        .any(|p| d.starts_with(p)) =>
                {
                    format!("refs/{d}")
                }
                (d, named) => match known(repo, fetch, remote, d, named.as_deref())? {
                    Some(full) => full,
                    None => return Ok(None),
                },
            }
        };
        out.push((src, full));
    }
    Ok(Some(out))
}

/// Where a spelled bare `name` lands, as git resolves it against the remote's refs with what
/// this clone holds: the remote's branch when its remote-tracking ref exists; else the tag
/// namespace when the source is a tag; else `None` when a local tag of that name leaves the
/// remote's unknown (git asks the remote, this clone cannot); else a new branch.
fn known(
    repo: &Path,
    fetch: &str,
    remote: &str,
    name: &str,
    source: Option<&str>,
) -> Result<Option<String>, String> {
    let seen = |r: &str| -> Result<bool, String> {
        let args = ["rev-parse", "--verify", "--quiet", "--end-of-options", r];
        let out = repo::probe(repo, &args, 1).map_err(|e| e.to_string())?;
        Ok(out.is_some())
    };
    let branch = format!("refs/heads/{name}");
    if let Some(tracked) = tracking(fetch, remote, &branch)
        && seen(&tracked)?
    {
        return Ok(Some(branch));
    }
    let tag = format!("refs/tags/{name}");
    if source.is_some_and(|f| f.starts_with("refs/tags/")) {
        return Ok(Some(tag));
    }
    Ok((!seen(&tag)?).then_some(branch))
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

/// `given` as a remote's name: itself when configured; else the remote whose `url` or
/// `pushurl` spells it, raw or through `insteadOf`, a trailing slash allowed for, a fetched
/// one first when two share it, so a push by URL still finds the old tip; else itself,
/// which no fetch refspec maps. Every word reaches git after `--end-of-options`.
fn named(repo: &Path, given: &str) -> Result<String, String> {
    let get = |args: &[&str]| -> Result<String, String> {
        let out = repo::probe(repo, args, 1).map_err(|e| e.to_string())?;
        Ok(out.unwrap_or_default())
    };
    if !get(&["config", "--get-all", &format!("remote.{given}.url")])?.is_empty() {
        return Ok(given.to_owned());
    }
    let trim = |u: &str| u.trim_end_matches('/').to_owned();
    let expand =
        |r: &str| get(&["ls-remote", "--get-url", "--end-of-options", r]).map(|u| trim(&u));
    let wants = [trim(given), expand(given)?];
    let mut unfetched = None;
    for name in get(&["remote"])?.lines() {
        let mut urls = vec![expand(name)?];
        for key in ["url", "pushurl"] {
            let listed = get(&["config", "--get-all", &format!("remote.{name}.{key}")])?;
            urls.extend(listed.lines().map(trim));
        }
        if !urls.iter().any(|u| wants.contains(u)) {
            continue;
        }
        let refs = format!("refs/remotes/{name}/");
        if !get(&["for-each-ref", "--count=1", &refs])?.is_empty() {
            return Ok(name.to_owned());
        }
        unfetched.get_or_insert_with(|| name.to_owned());
    }
    Ok(unfetched.unwrap_or_else(|| given.to_owned()))
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
