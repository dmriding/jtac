//! The roots that travel ahead of a push (DESIGN.md sections 9.6, 11.3): every commit in a
//! pushed range with a root in this clone, plus the tree match that covered a tip, sent in a
//! `git push` of their own with the hook's guard set. A root push that fails warns and never
//! refuses: the code push proceeds (decision 8).

use std::collections::BTreeSet;
use std::fmt::Write;
use std::path::Path;

use jtac_checks::git::git;
use jtac_core::GitOid;
use jtac_store::{PushOutcome, Store, ref_name};

use crate::repo::oid;

/// The variable the hook reads as "run nothing", set on its own root push.
pub const GUARD: &str = "JTAC_HOOK";

/// One ref that passed on a root.
pub struct Range {
    /// The pushed tip.
    pub tip: GitOid,
    /// The remote's old oid when this clone holds it; else the remote's tracking refs bound it.
    pub old: Option<GitOid>,
    /// The commit whose root covered the tip.
    pub counted: GitOid,
}

/// Push the roots for `ranges` to `remote`. Returns what to print on stderr.
pub fn push(repo: &Path, store: &Store, remote: &str, format: &str, ranges: &[Range]) -> String {
    if ranges.is_empty() {
        return String::new();
    }
    let commits = match commits(repo, store, remote, format, ranges) {
        Ok(commits) => commits,
        Err(why) => return warning(remote, &[], &why),
    };
    let pushed = match store.push(remote, &commits, &[(GUARD, "1")]) {
        Ok(pushed) => pushed,
        Err(e) => {
            let names: Vec<String> = commits.iter().map(ref_name).collect();
            return warning(remote, &names, &e.to_string());
        }
    };
    let rejected: Vec<String> = pushed
        .iter()
        .filter_map(|p| match &p.outcome {
            PushOutcome::Rejected { summary } => Some(format!("{} {summary}", p.name)),
            PushOutcome::Sent | PushOutcome::UpToDate => None,
        })
        .collect();
    if !rejected.is_empty() {
        return warning(remote, &rejected, "the remote refused them");
    }
    let sent = pushed
        .iter()
        .filter(|p| p.outcome == PushOutcome::Sent)
        .count();
    match sent {
        0 => String::new(),
        n => format!("jtac: sent {n} root(s) to {remote}\n"),
    }
}

/// Commits in each range that have a root here, then the commit whose root covered its tip.
fn commits(
    repo: &Path,
    store: &Store,
    remote: &str,
    format: &str,
    ranges: &[Range],
) -> Result<Vec<GitOid>, String> {
    let rooted: BTreeSet<GitOid> = store
        .roots()
        .map_err(|e| e.to_string())?
        .listed
        .into_iter()
        .map(|l| l.commit)
        .collect();
    let not_remote = format!("--remotes={remote}");
    let mut out = Vec::new();
    for r in ranges {
        let tip = r.tip.hex();
        let old = r.old.map(|o| format!("^{}", o.hex()));
        let bound = match &old {
            Some(old) => vec![old.as_str()],
            None => vec!["--not", not_remote.as_str()],
        };
        let mut args = vec!["rev-list", tip.as_str()];
        args.extend(bound);
        args.push("--");
        let listed = git(repo, &args).map_err(|e| e.to_string())?;
        for hex in String::from_utf8_lossy(&listed).lines() {
            let commit = oid(format, hex).map_err(|e| e.to_string())?;
            if rooted.contains(&commit) {
                out.push(commit);
            }
        }
        out.push(r.counted);
    }
    Ok(out)
}

/// Loud, in three parts, and never a refusal.
fn warning(remote: &str, refs: &[String], why: &str) -> String {
    let mut s = format!("jtac: WARNING: the roots did not reach {remote}: {why}.\n");
    for r in refs {
        let _ = writeln!(s, "jtac:   {r}");
    }
    let _ = writeln!(
        s,
        "The code push proceeds: a root push never blocks it. Until the roots arrive, every \
         reader sees these commits as absent; a rejected root most often means the remote \
         already holds another root for that commit, which stands."
    );
    let _ = writeln!(
        s,
        "next: git push {remote} 'refs/jtac/roots/*:refs/jtac/roots/*'"
    );
    s
}
