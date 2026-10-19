//! `jtac hook pre-tool <agent>` (DESIGN.md section 11.3): the agent's proposed command read
//! into actions, each push judged where it would run by the decision the `pre-push` hook
//! asks, and the note handed back on the agent's wire. It advises and never blocks: every
//! failure prints nothing and exits 0, and the work runs on a thread the main one abandons
//! at the cap, so a slow script costs the agent one second, not a hook timeout.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::Duration;

use jtac_gate::Decision;
use jtac_gate::classify::{Dir, Push, classify};
use jtac_store::Store;

use crate::wire::claude;
use crate::{coverage, repo, update};

/// The wall clock the hook gives itself (decided 2026-10-19).
pub const CAP: Duration = Duration::from_millis(1000);

/// Stdout and stderr; the exit code is always 0.
pub fn pre_tool(agent: &str, cwd: &Path, stdin: &Result<String, String>) -> (String, String) {
    if agent != claude::NAME {
        let why = format!(
            "jtac: no adapter for `{agent}`; the one built is `{}`\n",
            claude::NAME
        );
        return (String::new(), why);
    }
    let stdin = match stdin {
        Ok(text) => text.clone(),
        Err(why) => return (String::new(), format!("jtac: pre-tool: stdin: {why}\n")),
    };
    let cwd = cwd.to_path_buf();
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(advise(&stdin, &cwd));
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

/// One push's decision, or `None` when where it runs is not named or not a repository. The
/// first refused refspec is the answer; the remote's old tip is the remote-tracking ref,
/// the best this clone knows, as `status` reads it.
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
    let format = repo::format(&repo).map_err(|e| fail(&e))?;
    let store = Store::open(&repo).map_err(|e| fail(&e))?;
    let tool = coverage::verifier(&repo).map_err(|r| fail(&r.what))?;
    let remote = match &push.remote {
        Some(r) => r.clone(),
        None => remote(&repo).map_err(|e| fail(&e))?,
    };
    let mut last = None;
    for (src, dst) in targets(&repo, push).map_err(|e| fail(&e))? {
        let tip = match src {
            Some(rev) => Some(repo::peel(&repo, &format, &rev).map_err(|e| fail(&e))?),
            None => None,
        };
        let old = dst
            .strip_prefix("refs/heads/")
            .and_then(|b| repo::peel(&repo, &format, &format!("refs/remotes/{remote}/{b}")).ok())
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

/// `(source, full remote ref)` per refspec. No refspec, every ref, or a word the shell
/// built reads as HEAD to the branch it merges with, as `status` judges a bare `git push`.
fn targets(repo: &Path, push: &Push) -> Result<Vec<(Option<String>, String)>, String> {
    if push.all_refs || push.opaque || push.destinations.is_empty() {
        return Ok(vec![(Some("HEAD".to_owned()), merge_ref(repo)?)]);
    }
    let mut out = Vec::new();
    for (i, dst) in push.destinations.iter().enumerate() {
        let src = (!push.delete).then(|| push.sources.get(i).cloned().unwrap_or_default());
        let full = match dst.as_str() {
            "HEAD" => merge_ref(repo)?,
            d if d.starts_with("refs/") => d.to_owned(),
            d => match d.strip_prefix("tag ") {
                Some(tag) => format!("refs/tags/{tag}"),
                None => format!("refs/heads/{d}"),
            },
        };
        out.push((src, full));
    }
    Ok(out)
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

/// The ref HEAD's branch merges with, else the branch itself, else `HEAD`.
fn merge_ref(repo: &Path) -> Result<String, String> {
    let Some(branch) = short_branch(repo)? else {
        return Ok("HEAD".to_owned());
    };
    let merge = repo::config(repo, &format!("branch.{branch}.merge")).map_err(|e| e.to_string())?;
    Ok(merge.unwrap_or_else(|| format!("refs/heads/{branch}")))
}

fn short_branch(repo: &Path) -> Result<Option<String>, String> {
    let full =
        repo::probe(repo, &["symbolic-ref", "--quiet", "HEAD"], 1).map_err(|e| e.to_string())?;
    Ok(full.and_then(|f| f.strip_prefix("refs/heads/").map(str::to_owned)))
}
