//! `jtac status` (DESIGN.md sections 7.1, 11.9): whether a push of HEAD to its upstream
//! would pass, asked of the decision the `pre-push` hook asks, the unpushed commits with their
//! coverage, the waivers in effect for HEAD, and the last verdict.

use std::fmt::Write;
use std::path::Path;

use jtac_core::{GitOid, Sha256Digest, Ts, Waivable, scope_at};
use jtac_gate::{Decision, Refuse};
use jtac_store::Store;
use serde::Serialize;

use crate::output::WaiverOut;
use crate::refuse::Refusal;
use crate::{Printed, chain, coverage, repo, state, update};

/// What `jtac status` prints.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct StatusOut {
    /// Output schema version.
    pub v: u32,
    /// The command.
    pub command: &'static str,
    /// Whether a push of HEAD to its upstream would pass.
    pub pass: bool,
    /// HEAD.
    pub commit: GitOid,
    /// HEAD's root, when it has one that counts.
    pub root: Option<Sha256Digest>,
    /// The commit whose root judged HEAD's tree, when HEAD has none of its own.
    pub tree_match: Option<GitOid>,
    /// The branch the unpushed commits are counted against, when there is one.
    pub upstream: Option<String>,
    /// Commits on HEAD and not on the upstream, oldest first.
    pub unpushed: Vec<Unpushed>,
    /// Stored waivers a `jtac check` on HEAD now would apply.
    pub waivers: Vec<WaiverOut>,
    /// The last verdict, when a check has run in this clone.
    pub last: Option<Last>,
    /// What could not be read, such as a `state.json` another version wrote.
    pub notes: Vec<String>,
    /// Why a push would be refused: `rewind` (the fence), `unknown`, or `absent`.
    pub refused: Option<&'static str>,
    /// The decision, which the text renders.
    #[serde(skip)]
    pub decision: Decision,
}

/// One commit the upstream does not have.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Unpushed {
    /// The commit.
    pub commit: GitOid,
    /// Whether it has a root of its own.
    pub covered: bool,
}

/// The last verdict in brief.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Last {
    /// The commit it judged.
    pub commit: GitOid,
    /// When.
    pub ts: Ts,
    /// `CLEARED HOT` or `ABORT`.
    pub banner: String,
    /// The root it wrote or found.
    pub root: Option<Sha256Digest>,
}

/// Gather and render. Exit 0 when a push would pass, 1 when it would be refused. HEAD goes
/// to the branch it merges with (`branch.<name>.merge`), else the one it is on, over the
/// upstream's tip; with no upstream it creates the branch.
pub fn status(cwd: &Path, json: bool) -> Result<Printed, Refusal> {
    let repo = repo::root(cwd).map_err(|e| Refusal::repository(&e))?;
    if let Some(path) = repo::grafted(&repo).map_err(|e| Refusal::repository(&e))? {
        return Err(Refusal::grafted(&path));
    }
    let format = repo::format(&repo).map_err(|e| Refusal::repository(&e))?;
    let head = repo::peel(&repo, &format, "HEAD").map_err(|e| Refusal::no_head(&e))?;
    let store = Store::open(&repo).map_err(Refusal::store)?;
    let upstream = repo::upstream(&repo).map_err(|e| Refusal::repository(&e))?;
    let old = match &upstream {
        Some(full) => Some(
            repo::peel(&repo, &format, full)
                .map_err(|e| Refusal::repository(&e))?
                .commit,
        ),
        None => None,
    };
    let branch = repo::probe(&repo, &["symbolic-ref", "--quiet", "HEAD"], 1)
        .map_err(|e| Refusal::repository(&e))?
        .unwrap_or_else(|| "HEAD".to_owned());
    let merge = branch
        .strip_prefix("refs/heads/")
        .map(|short| repo::config(&repo, &format!("branch.{short}.merge")))
        .transpose()
        .map_err(|e| Refusal::repository(&e))?
        .flatten();
    let remote_ref = merge.as_deref().unwrap_or(&branch);
    let tool = coverage::verifier(&repo)?;
    let g = update::gather(
        &repo,
        &store,
        &tool,
        &format,
        Some(&head),
        remote_ref,
        old.as_ref(),
    )?;
    let decision = jtac_gate::decide(&g.update);
    let (root, tree_match) = match g.tip.counted {
        Some((commit, id)) if commit == head.commit => (Some(id), None),
        Some((commit, _)) => (None, Some(commit)),
        None => (None, None),
    };
    let roots = store.roots().map_err(Refusal::store)?;
    let covered = chain::covered(&repo, &roots, &head)?;
    let unpushed = match &upstream {
        Some(full) => repo::ahead(&repo, &head, full)
            .map_err(|e| Refusal::repository(&e))?
            .into_iter()
            .map(|commit| Unpushed {
                covered: (commit == head.commit && root.is_some()) || covered.contains(&commit),
                commit,
            })
            .collect(),
        None => Vec::new(),
    };
    let now = Ts::from_timestamp(jiff::Timestamp::now()).map_err(Refusal::internal)?;
    let mut notes = g.tip.notes;
    let waivers = crate::lever::offered(&repo, &store, &mut notes)?
        .iter()
        .filter(|l| scope_at(&l.node, &head.commit, Waivable::Tip, now).is_ok())
        .filter_map(WaiverOut::of)
        .collect();
    let last = match state::read(&store) {
        Ok(last) => last.map(|o| Last {
            commit: o.commit,
            ts: o.ts,
            banner: o.banner,
            root: o.root,
        }),
        Err(r) => {
            notes.push(r.what);
            None
        }
    };
    let out = StatusOut {
        v: 1,
        command: "status",
        pass: matches!(decision, Decision::Pass(_)),
        commit: head.commit,
        root,
        tree_match,
        upstream: upstream.map(|u| u.trim_start_matches("refs/remotes/").to_owned()),
        unpushed,
        waivers,
        last,
        notes,
        refused: match &decision {
            Decision::Pass(_) => None,
            Decision::Refuse(Refuse::Rewind | Refuse::Delete) => Some("rewind"),
            Decision::Refuse(Refuse::Unknown) => Some("unknown"),
            Decision::Refuse(Refuse::Absent) => Some("absent"),
        },
        decision,
    };
    let text = if json {
        crate::render::json(&out)?
    } else {
        render(&out)
    };
    Ok(Printed::verdict(text, out.pass))
}

fn render(o: &StatusOut) -> String {
    let mut s = String::new();
    let head = o.commit.hex();
    match (&o.decision, &o.root, &o.tree_match) {
        (Decision::Pass(_), Some(root), _) => {
            let _ = writeln!(s, "PUSH WOULD PASS: HEAD {head} has root {root}");
        }
        (Decision::Pass(_), None, Some(other)) => {
            let _ = writeln!(
                s,
                "PUSH WOULD PASS: HEAD {head} has no root of its own; {}'s root judged the \
                 same tree",
                other.hex()
            );
        }
        (Decision::Refuse(Refuse::Rewind | Refuse::Delete), ..) => {
            let _ = writeln!(
                s,
                "PUSH WOULD BE REFUSED: HEAD {head} rewinds its upstream, a protected branch"
            );
            let _ = writeln!(
                s,
                "  fence   `[fences].force_push_protected` is `deny`; no command clears it from \
                 here, and a bigger `--force` does not either"
            );
        }
        (Decision::Refuse(Refuse::Unknown), ..) => {
            let _ = writeln!(
                s,
                "PUSH WOULD BE REFUSED: the upstream's tip is not in this clone"
            );
            let _ = writeln!(s, "  fix     run `git fetch`");
        }
        _ => {
            let _ = writeln!(s, "PUSH WOULD BE REFUSED: HEAD {head} has no root");
            let _ = writeln!(s, "  fix     run `jtac check`");
        }
    }
    match &o.upstream {
        Some(up) if o.unpushed.is_empty() => {
            let _ = writeln!(s, "  upstream {up}: nothing to push");
        }
        Some(up) => {
            let covered = o.unpushed.iter().filter(|u| u.covered).count();
            let _ = writeln!(
                s,
                "  upstream {up}: {} unpushed, {covered} covered, {} absent",
                o.unpushed.len(),
                o.unpushed.len() - covered
            );
            for u in &o.unpushed {
                let state = if u.covered { "covered" } else { "absent" };
                let _ = writeln!(s, "     {} {state}", u.commit.hex());
            }
        }
        None => {
            let _ = writeln!(s, "  upstream none: unpushed commits are not counted");
        }
    }
    for w in &o.waivers {
        let _ = writeln!(s, "  waiver  {}", w.line());
    }
    for note in &o.notes {
        let _ = writeln!(s, "  note    {note}");
    }
    match &o.last {
        Some(l) => {
            let _ = writeln!(
                s,
                "  last    {} at {} on {}",
                l.banner,
                l.ts,
                l.commit.hex()
            );
        }
        None => {
            let _ = writeln!(s, "  last    no check has run in this clone");
        }
    }
    s
}
