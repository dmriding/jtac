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
use jtac_store::{PushOutcome, Pushed, Space, Store, StoreError, ref_name};

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
        Err(why) => return warning(remote, &[], &[], &why),
    };
    out.push_str(&send(store, remote, &commits).text);
    out
}

/// What a root push left: the stderr text, and the refs the remote holds afterwards.
pub struct Sent {
    /// For stderr.
    pub text: String,
    /// Sent or already there; a rejected ref is not among them.
    pub held: Vec<String>,
    /// The refs the remote refused.
    pub rejected: Vec<String>,
}

/// Push the root refs of `commits` to `remote` under the hook's guard.
pub fn send(store: &Store, remote: &str, commits: &[GitOid]) -> Sent {
    let names: Vec<String> = commits.iter().map(ref_name).collect();
    report(remote, &names, store.push(remote, commits, &[(GUARD, "1")]))
}

/// Push `names` as `send` pushes roots; the merge job's route, whose tip's document may be
/// a failure record (DESIGN.md sections 15 and 15.2).
pub fn send_refs(store: &Store, remote: &str, names: &[String]) -> Sent {
    report(
        remote,
        names,
        store.push_refs(remote, names, &[(GUARD, "1")]),
    )
}

fn report(remote: &str, names: &[String], pushed: Result<Vec<Pushed>, StoreError>) -> Sent {
    let mut text = String::new();
    let pushed = match pushed {
        Ok(pushed) => pushed,
        Err(e) => {
            text.push_str(&warning(remote, names, names, &e.to_string()));
            return Sent {
                text,
                held: Vec::new(),
                rejected: Vec::new(),
            };
        }
    };
    let (refused, kept): (Vec<&Pushed>, Vec<&Pushed>) = pushed
        .iter()
        .partition(|p| matches!(p.outcome, PushOutcome::Rejected { .. }));
    let held = kept.iter().map(|p| p.name.clone()).collect();
    let rejected: Vec<String> = refused.iter().map(|p| p.name.clone()).collect();
    if !refused.is_empty() {
        let lines: Vec<String> = refused
            .iter()
            .filter_map(|p| match &p.outcome {
                PushOutcome::Rejected { summary } => Some(format!("{} {summary}", p.name)),
                PushOutcome::Sent | PushOutcome::UpToDate => None,
            })
            .collect();
        text.push_str(&warning(remote, names, &lines, "the remote refused them"));
        return Sent {
            text,
            held,
            rejected,
        };
    }
    let sent = |record: bool| {
        (kept.iter())
            .filter(|p| p.outcome == PushOutcome::Sent)
            .filter(|p| p.name.starts_with(Space::Failed.prefix()) == record)
            .count()
    };
    if sent(false) > 0 {
        let _ = writeln!(text, "jtac: sent {} root(s) to {remote}", sent(false));
    }
    if sent(true) > 0 {
        let _ = writeln!(
            text,
            "jtac: sent {} failure record(s) to {remote}",
            sent(true)
        );
    }
    Sent {
        text,
        held,
        rejected,
    }
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

/// Loud, in three parts, and never a refusal. `names` are the refs the push carried and
/// `lines` what to print of them; the next step names each ref when one is no root's,
/// since the roots' pattern would leave a failure record behind.
fn warning(remote: &str, names: &[String], lines: &[String], why: &str) -> String {
    let mut s = format!("jtac: WARNING: the roots did not reach {remote}: {why}.\n");
    for r in lines {
        let _ = writeln!(s, "jtac:   {r}");
    }
    let roots = Space::Roots.prefix();
    let spec = match names.iter().all(|n| n.starts_with(roots)) {
        true => format!("'{roots}*:{roots}*'"),
        false => names.join(" "),
    };
    let _ = writeln!(
        s,
        "The code push proceeds: a root push never blocks it. Until the roots arrive, every \
         reader sees these commits as absent; a rejected root most often means the remote \
         already holds another root for that commit, which stands."
    );
    let _ = writeln!(s, "next: git push {} {spec}", quoted(remote, "<remote>"));
    s
}
