//! `jtac status` (DESIGN.md section 7.1): whether a push would pass, read the way the
//! `pre-push` hook will read it, the unpushed commits with their coverage, and the last
//! verdict. Waivers arrive with `jtac waive`.

use std::fmt::Write;
use std::path::Path;

use jtac_core::{GitOid, Sha256Digest, Ts};
use jtac_store::Store;
use serde::Serialize;

use crate::refuse::Refusal;
use crate::{Printed, chain, repo, state};

/// What `jtac status` prints.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct StatusOut {
    /// Output schema version.
    pub v: u32,
    /// The command.
    pub command: &'static str,
    /// Whether HEAD has a root, so a push would pass.
    pub pass: bool,
    /// HEAD.
    pub commit: GitOid,
    /// HEAD's root, when it has one.
    pub root: Option<Sha256Digest>,
    /// The branch the unpushed commits are counted against, when there is one.
    pub upstream: Option<String>,
    /// Commits on HEAD and not on the upstream, oldest first.
    pub unpushed: Vec<Unpushed>,
    /// The last verdict, when a check has run in this clone.
    pub last: Option<Last>,
    /// What could not be read, such as a `state.json` another version wrote.
    pub notes: Vec<String>,
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

/// Gather and render. Exit 0 when a push would pass, 1 when it would be refused.
pub fn status(cwd: &Path, json: bool) -> Result<Printed, Refusal> {
    let repo = repo::root(cwd).map_err(|e| Refusal::repository(&e))?;
    let head = repo::head(&repo).map_err(|e| Refusal::no_head(&e))?;
    let store = Store::open(&repo).map_err(Refusal::store)?;
    let root = store
        .root(&head.commit)
        .map_err(Refusal::store)?
        .map(|s| s.root.id())
        .transpose()
        .map_err(Refusal::internal)?;
    let covered = chain::covered(&repo, &store, &head)?;
    let upstream = repo::upstream(&repo).map_err(|e| Refusal::repository(&e))?;
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
    let mut notes = Vec::new();
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
        pass: root.is_some(),
        commit: head.commit,
        root,
        upstream: upstream.map(|u| u.trim_start_matches("refs/remotes/").to_owned()),
        unpushed,
        last,
        notes,
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
    match &o.root {
        Some(root) => {
            let _ = writeln!(
                s,
                "PUSH WOULD PASS: HEAD {} has root {root}",
                o.commit.hex()
            );
        }
        None => {
            let _ = writeln!(
                s,
                "PUSH WOULD BE REFUSED: HEAD {} has no root",
                o.commit.hex()
            );
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
