//! `jtac hook pre-push` (DESIGN.md section 11.3): git's stdin lines to ref updates, the facts
//! around each, the pure decision, then the root refs pushed ahead of the code. Nothing here
//! writes to stdout; a hook that cannot answer refuses with exit 2.

use std::fmt::Write;
use std::path::{Path, PathBuf};

use jtac_core::{GitOid, State};
use jtac_gate::{Decision, History, Pass, Refuse};
use jtac_sign::Tool;
use jtac_store::Store;

use crate::refuse::Refusal;
use crate::rootpush::{GUARD, Range};
use crate::{coverage, repo, rootpush, update};

/// What the hook wrote to stderr and how it exited: 0 lets the push proceed, 1 refuses it,
/// 2 could not answer, which also refuses. Stdin that could not be read is 2: an unread
/// line is an unjudged ref (`non_utf8_stdin_refuses_rather_than_passes`).
pub fn pre_push(
    cwd: &Path,
    remote: &str,
    _url: &str,
    stdin: &Result<String, String>,
) -> (String, u8) {
    if std::env::var_os(GUARD).is_some_and(|v| v == "1") {
        return (String::new(), 0);
    }
    let stdin = match stdin {
        Ok(text) => text,
        Err(why) => return (Refusal::hook_stdin(why).text(), 2),
    };
    match judge(cwd, remote, stdin) {
        Ok(out) => out,
        Err(r) => (r.text(), 2),
    }
}

/// One line git wrote: `<local ref> <local oid> <remote ref> <remote oid>`.
#[derive(Debug, PartialEq, Eq)]
struct Line<'a> {
    raw: &'a str,
    local: &'a str,
    remote_ref: &'a str,
    old: &'a str,
}

/// Blank lines are skipped; any other line that is not four fields refuses. Split from the
/// right: only the local ref, a user's own expression, can hold a space.
fn lines(stdin: &str) -> Result<Vec<Line<'_>>, Refusal> {
    stdin
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| match l.rsplitn(4, ' ').collect::<Vec<_>>()[..] {
            [old, remote_ref, local, _] => Ok(Line {
                raw: l,
                local,
                remote_ref,
                old,
            }),
            _ => Err(Refusal::hook_line(l)),
        })
        .collect()
}

/// Where a refused ref was going, for the message.
struct At<'a> {
    remote: &'a str,
    remote_ref: &'a str,
    tip: &'a str,
    old: &'a str,
    /// The tip's own root ref, when it exists and did not count.
    stale: Option<String>,
}

/// The mirror check comes before the empty-stdin pass: a mirror whose only work is deletion
/// writes no line at all (`a_mirror_push_is_refused_because_git_hides_its_deletions`).
fn judge(cwd: &Path, remote: &str, stdin: &str) -> Result<(String, u8), Refusal> {
    let lines = lines(stdin)?;
    let repo = repo::root(cwd).map_err(|e| Refusal::repository(&e))?;
    let key = format!("remote.{remote}.mirror");
    let mirror = repo::probe(&repo, &["config", "--type=bool", "--get", &key], 1)
        .map_err(|e| Refusal::repository(&e))?;
    if mirror.as_deref() == Some("true") {
        return Err(Refusal::mirror(remote));
    }
    if lines.is_empty() {
        return Ok((String::new(), 0));
    }
    if let Some(path) = repo::grafted(&repo).map_err(|e| Refusal::repository(&e))? {
        return Err(Refusal::grafted(&path));
    }
    let format = repo::format(&repo).map_err(|e| Refusal::repository(&e))?;
    let ctx = Ctx {
        store: Store::open(&repo).map_err(Refusal::store)?,
        tool: coverage::verifier(&repo)?,
        repo,
        format,
    };
    let mut out = String::new();
    let mut refused = false;
    let mut ranges = Vec::new();
    for line in &lines {
        let (decision, stale) = ctx.decide(line, &mut out, &mut ranges)?;
        let at = At {
            remote,
            remote_ref: line.remote_ref,
            tip: line.local,
            old: line.old,
            stale,
        };
        match decision {
            Decision::Pass(p) => {
                let _ = writeln!(out, "jtac: {}: {}", line.remote_ref, passed(&p, line.local));
            }
            Decision::Refuse(r) if refused => {
                let _ = writeln!(out, "jtac: {}", Refusal::push(&r, &at).what);
            }
            Decision::Refuse(r) => {
                refused = true;
                out.push_str(&Refusal::push(&r, &at).text());
            }
        }
    }
    if refused {
        return Ok((out, 1));
    }
    out.push_str(&rootpush::push(
        &ctx.repo,
        &ctx.store,
        remote,
        &ctx.format,
        &ranges,
    ));
    Ok((out, 0))
}

/// What every line is judged with.
struct Ctx {
    repo: PathBuf,
    format: String,
    store: Store,
    tool: Tool,
}

impl Ctx {
    /// One line's decision, with the tip's own root ref when it exists and did not count.
    /// Notes go to `out`; a ref that passed on a root adds its range.
    fn decide(
        &self,
        line: &Line,
        out: &mut String,
        ranges: &mut Vec<Range>,
    ) -> Result<(Decision, Option<String>), Refusal> {
        let local = self.oid(line.local, line)?;
        let old = self.oid(line.old, line)?;
        if let Some(pass) = jtac_gate::exempt(line.remote_ref, local.is_none()) {
            return Ok((Decision::Pass(pass), None));
        }
        let at_ref = |r: Refusal| Refusal {
            what: format!("`{}`: {}", line.remote_ref, r.what),
            ..r
        };
        let tip = local
            .map(|l| repo::peel(&self.repo, &self.format, &l.hex()))
            .transpose()
            .map_err(|e| at_ref(Refusal::repository(&e)))?;
        let g = update::gather(
            &self.repo,
            &self.store,
            &self.tool,
            &self.format,
            tip.as_ref(),
            line.remote_ref,
            old.as_ref(),
        )
        .map_err(at_ref)?;
        for note in &g.tip.notes {
            let _ = writeln!(out, "jtac: note: {note}");
        }
        let decision = jtac_gate::decide(&g.update);
        if let (Decision::Pass(_), Some((counted, _)), Some(tip)) = (&decision, g.tip.counted, &tip)
        {
            let known = matches!(g.update.history, History::FastForward | History::Rewinds);
            ranges.push(Range {
                tip: tip.commit,
                old: old.filter(|_| known),
                counted,
            });
        }
        let stale = tip
            .filter(|_| g.tip.own_stale)
            .map(|t| jtac_store::ref_name(&t.commit));
        Ok((decision, stale))
    }

    /// `None` for the null oid, in this repository's format.
    fn oid(&self, hex: &str, line: &Line) -> Result<Option<GitOid>, Refusal> {
        let oid = repo::oid(&self.format, hex).map_err(|_| Refusal::hook_line(line.raw))?;
        Ok((!oid.as_bytes().iter().all(|b| *b == 0)).then_some(oid))
    }
}

/// The one line a passing ref gets.
fn passed(p: &Pass, tip: &str) -> String {
    match p {
        Pass::Delete => "a deletion needs no evidence".to_owned(),
        Pass::Tag => "a tag needs no evidence".to_owned(),
        Pass::Evidence => "evidence, not code".to_owned(),
        Pass::Root(State::Covered) => format!("{tip} has a root, covered"),
        Pass::Root(State::DeclaredOff) => {
            format!("{tip} has a root, declared off: a waiver turned the gate off for it")
        }
        Pass::Tree { commit } => {
            format!("{tip} has no root of its own; {commit}'s root judged the same tree")
        }
    }
}

impl Refusal {
    /// Stdin could not be read as text, so no line can be judged.
    fn hook_stdin(why: &str) -> Self {
        Self {
            kind: "negotiation",
            what: format!("`jtac hook pre-push` could not read its stdin: {why}."),
            why: "The hook judges every ref git is about to push; a line it cannot read is a ref \
                  it cannot judge, and an unjudged ref must not pass."
                .to_owned(),
            how: "Ref names this release reads are UTF-8: rename the ref with `git branch -m \
                  <old> <new>`, then push again."
                .to_owned(),
        }
    }

    /// A mirror remote: git deletes what the remote has and the clone lacks without a line on
    /// the hook's stdin, so a protected branch would go unjudged.
    fn mirror(remote: &str) -> Self {
        Self {
            kind: "negotiation",
            what: format!("`remote.{remote}.mirror` is set: every push to `{remote}` mirrors."),
            why: "A mirror push deletes every remote ref the clone lacks, and git never writes \
                  those deletions to the pre-push hook; a protected branch could vanish \
                  unjudged, so the whole push is refused."
                .to_owned(),
            how: format!(
                "Run `git config --unset remote.{remote}.mirror`, then push branches by name, \
                 `git push {remote} <branch>`, and delete by name, `git push {remote} \
                 :<branch>`, which the hook judges."
            ),
        }
    }

    /// A line on the hook's stdin that is not what git writes.
    fn hook_line(line: &str) -> Self {
        Self {
            kind: "negotiation",
            what: format!("`jtac hook pre-push` cannot read the line `{line}`."),
            why: "The hook judges every ref git is about to push; a line it cannot read is a ref \
                  it cannot judge, and an unjudged ref must not pass."
                .to_owned(),
            how: "Push with `git push`, which writes `<local ref> <local oid> <remote ref> \
                  <remote oid>` lines; do not run `jtac hook pre-push` by hand."
                .to_owned(),
        }
    }

    /// A ref the decision refused.
    fn push(r: &Refuse, at: &At) -> Self {
        let branch = at.remote_ref.trim_start_matches("refs/heads/");
        let (what, why, how) = match r {
            Refuse::Absent => (
                format!(
                    "`{}`: {} has no root, and no root judged its tree.",
                    at.remote_ref, at.tip
                ),
                "The pre-push hook is the gate: a commit leaves this clone with signed evidence \
                 that the checks ran on its tree, or it does not leave."
                    .to_owned(),
                match &at.stale {
                    Some(stale) => format!(
                        "The root ref {stale} is there and does not count (see the note \
                         above), and `jtac check` keeps a root it finds: run `git update-ref \
                         -d {stale}`, then `jtac check` with {} checked out, then push again. \
                         Do not add `--no-verify`: the commit would land with no root and read \
                         as absent to every reader.",
                        at.tip
                    ),
                    None => format!(
                        "Run `jtac check` with {} checked out, then push again. Do not add \
                         `--no-verify`: the commit would land with no root and read as absent \
                         to every reader.",
                        at.tip
                    ),
                },
            ),
            Refuse::Unknown => (
                format!(
                    "`{}`: the remote's tip {} is not in this clone.",
                    at.remote_ref, at.old
                ),
                format!(
                    "The branch's own `.jtac/policy.toml` at that tip is half the fence that \
                     protects it, and a rewind cannot be ruled out against a commit this clone \
                     has never seen; a tip must not vouch for `{branch}` alone."
                ),
                format!(
                    "Run `git fetch {} {branch}`, then push again; if it is then refused as a \
                     rewind, merge or rebase onto the fetched tip first.",
                    at.remote
                ),
            ),
            Refuse::Delete => (
                format!(
                    "`{}`: this push deletes a protected branch at {}.",
                    at.remote_ref, at.old
                ),
                "`[fences].force_push_protected` is `deny` in `.jtac/policy.toml` at the \
                 branch's tip: deleting it and pushing it again would rewind it past the \
                 fence, so the deletion is the fence."
                    .to_owned(),
                format!(
                    "No command clears this from here, and `--force` does not either. Do not \
                     add `--no-verify`. What would clear it is `[fences]` in \
                     `.jtac/policy.toml` on `{branch}`, changed through review; to retire the \
                     branch, do that first, then `git push {} :{}`.",
                    at.remote, at.remote_ref
                ),
            ),
            Refuse::Rewind => (
                format!(
                    "`{}`: this push rewinds a protected branch: the remote's tip {} is not an \
                     ancestor of {}.",
                    at.remote_ref, at.old, at.tip
                ),
                "`[fences].force_push_protected` is `deny` in `.jtac/policy.toml` at the \
                 branch's tip or at yours, and either one holds: history published on a \
                 protected branch is never rewritten, because roots and reviews name its \
                 commits."
                    .to_owned(),
                format!(
                    "No command clears this from here, and a bigger `--force` does not either: \
                     the hook compares the remote's tip with yours, never the flags. Do not add \
                     `--no-verify`: the rewrite would land with no evidence. What would clear it \
                     is `[fences]` in `.jtac/policy.toml`, changed through review. To keep this \
                     work, push it to a branch of its own: `git push {} {}:refs/heads/{branch}-rework`.",
                    at.remote, at.tip
                ),
            ),
        };
        let kind = if r.is_fence() { "fence" } else { "negotiation" };
        Self {
            kind,
            what,
            why,
            how,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stdin_lines_are_four_fields_and_blank_lines_are_skipped() {
        let z = "0".repeat(40);
        let a = "a".repeat(40);
        let push = format!("refs/heads/main {a} refs/heads/main {z}");
        let delete = format!("(delete) {z} refs/heads/x {a}");
        let stdin = format!("{push}\n\n  \n{delete}\n");
        let got = lines(&stdin).unwrap();
        assert_eq!(
            got,
            vec![
                Line {
                    raw: &push,
                    local: &a,
                    remote_ref: "refs/heads/main",
                    old: &z
                },
                Line {
                    raw: &delete,
                    local: &z,
                    remote_ref: "refs/heads/x",
                    old: &a
                },
            ]
        );
        assert_eq!(lines("").unwrap(), vec![]);
        let spaced = format!("main@{{0 days ago}} {a} refs/heads/topic {z}");
        let got = lines(&spaced).unwrap();
        assert_eq!(
            (got[0].local, got[0].remote_ref, got[0].old),
            (a.as_str(), "refs/heads/topic", z.as_str())
        );
        for bad in [
            "refs/heads/main",
            "a b c",
            "refs/heads/main\taaa\trefs/heads/main\tbbb",
        ] {
            let err = lines(bad).unwrap_err();
            assert!(err.what.contains(bad), "{bad}: {err:?}");
        }
    }

    #[test]
    fn every_hook_refusal_has_three_parts_names_a_command_and_says_its_kind() {
        let (tip, old) = ("b".repeat(40), "a".repeat(40));
        let at = At {
            remote: "origin",
            remote_ref: "refs/heads/main",
            tip: &tip,
            old: &old,
            stale: None,
        };
        let stale = At {
            stale: Some("refs/jtac/roots/bb/b".into()),
            remote: at.remote,
            remote_ref: at.remote_ref,
            tip: at.tip,
            old: at.old,
        };
        let spawn = jtac_sign::SignError::Spawn {
            argv: vec!["ssh-keygen".into()],
            source: std::io::Error::other("nope"),
        };
        let rows = [
            (Refusal::hook_line("x"), "negotiation"),
            (Refusal::hook_stdin("it is not UTF-8"), "negotiation"),
            (Refusal::mirror("origin"), "negotiation"),
            (Refusal::verifier(&spawn), "negotiation"),
            (Refusal::push(&Refuse::Absent, &at), "negotiation"),
            (Refusal::push(&Refuse::Unknown, &at), "negotiation"),
            (Refusal::push(&Refuse::Rewind, &at), "fence"),
            (Refusal::push(&Refuse::Delete, &at), "fence"),
            (Refusal::push(&Refuse::Absent, &stale), "negotiation"),
        ];
        for (r, kind) in &rows {
            assert!(!r.what.is_empty() && !r.why.is_empty(), "{r:?}");
            assert!(r.how.contains("jtac ") || r.how.contains("git "), "{r:?}");
            assert_eq!(r.kind, *kind, "{r:?}");
            assert!(r.text().contains(&format!("\n{kind}: ")), "{r:?}");
        }
        assert!(
            Refusal::push(&Refuse::Absent, &at)
                .how
                .contains("jtac check")
        );
        let unknown = Refusal::push(&Refuse::Unknown, &at);
        assert!(unknown.how.contains("git fetch origin main"), "{unknown:?}");
        let fence = Refusal::push(&Refuse::Rewind, &at);
        assert!(fence.how.contains("--force") && fence.how.contains("[fences]"));
    }
}
