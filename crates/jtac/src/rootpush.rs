//! The roots that travel ahead of a push (DESIGN.md sections 9.6, 11.3): every commit in a
//! pushed range with a root in this clone, plus the tree match that covered a tip, sent in a
//! `git push` of their own with the hook's guard set. A root with `from` travels only as the
//! root that counted for a tip, since the hook judged no other and CI pushes its own
//! (section 15). A root push that fails warns and never refuses: the code push proceeds
//! (decision 8).

use std::collections::BTreeSet;
use std::fmt::Write;
use std::path::Path;

use jtac_checks::git::{git, quoted};
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
    let (commits, mut out) = match commits(repo, store, remote, format, ranges) {
        Ok(found) => found,
        Err(why) => return warning(remote, &[], &why),
    };
    out.push_str(&send(store, remote, &commits));
    out
}

/// Push the root refs of `commits` to `remote` under the hook's guard; the merge job's own
/// route (DESIGN.md section 15). Returns what to print on stderr.
pub fn send(store: &Store, remote: &str, commits: &[GitOid]) -> String {
    let mut out = String::new();
    let pushed = match store.push(remote, commits, &[(GUARD, "1")]) {
        Ok(pushed) => pushed,
        Err(e) => {
            let names: Vec<String> = commits.iter().map(ref_name).collect();
            out.push_str(&warning(remote, &names, &e.to_string()));
            return out;
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
        out.push_str(&warning(remote, &rejected, "the remote refused them"));
        return out;
    }
    let sent = pushed
        .iter()
        .filter(|p| p.outcome == PushOutcome::Sent)
        .count();
    if sent > 0 {
        let _ = writeln!(out, "jtac: sent {sent} root(s) to {remote}");
    }
    out
}

/// Commits in each range that have a root here, then the commit whose root covered its tip,
/// with a note for each root with `from` left behind.
fn commits(
    repo: &Path,
    store: &Store,
    remote: &str,
    format: &str,
    ranges: &[Range],
) -> Result<(Vec<GitOid>, String), String> {
    let rooted: BTreeSet<GitOid> = store
        .roots()
        .map_err(|e| e.to_string())?
        .listed
        .into_iter()
        .map(|l| l.commit)
        .collect();
    let not_remote = format!("--remotes={remote}");
    let counted: BTreeSet<GitOid> = ranges.iter().map(|r| r.counted).collect();
    let mut out = Vec::new();
    let mut notes = BTreeSet::new();
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
            if !rooted.contains(&commit) || counted.contains(&commit) {
                continue;
            }
            match store.root(&commit) {
                Ok(Some(s)) if s.root.from.is_none() => out.push(commit),
                Ok(Some(_)) => {
                    notes.insert(format!(
                        "jtac: note: {}'s root has `from` and was not sent: a CI root travels \
                         by CI's hand, or as the root that counted for the pushed tip\n",
                        commit.hex()
                    ));
                }
                Ok(None) => {}
                Err(e) => {
                    notes.insert(format!(
                        "jtac: note: {}'s root was not sent: {e}\n",
                        commit.hex()
                    ));
                }
            }
        }
        out.push(r.counted);
    }
    Ok((out, notes.into_iter().collect()))
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
        "next: git push {} 'refs/jtac/roots/*:refs/jtac/roots/*'",
        quoted(remote, "<remote>")
    );
    s
}
