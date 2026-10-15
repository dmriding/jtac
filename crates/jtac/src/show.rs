//! `jtac show` (DESIGN.md sections 7.1, 11.9 and 19): the root refs a push of the range would
//! send, each with the two blobs its commit carries, byte for byte. Nothing is verified here;
//! `verify` judges, `show` only prints what leaves the machine.

use std::fmt::Write;
use std::path::Path;

use jtac_core::GitOid;
use jtac_store::{Store, ref_name};
use serde::Serialize;

use crate::Printed;
use crate::cli::ShowArgs;
use crate::refuse::Refusal;
use crate::{range, repo};

/// What `jtac show` prints.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ShowOut {
    /// Output schema version.
    pub v: u32,
    /// The command.
    pub command: &'static str,
    /// The range as listed.
    pub range: Vec<String>,
    /// The roots a push would send, newest commit first.
    pub roots: Vec<Shown>,
    /// Commits in the range with no root ref of their own; a later root may still cover them.
    pub without_ref: Vec<GitOid>,
    /// Refs in the range whose content is not a root, and skipped names.
    pub notes: Vec<String>,
}

/// One root ref as the push sends it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Shown {
    /// The commit the root is for.
    pub commit: GitOid,
    /// The ref.
    pub name: String,
    /// The root commit the ref points at.
    pub root_commit: GitOid,
    /// `root.json`, the canonical bytes.
    pub root_json: String,
    /// `root.sig`, the armored signature.
    pub root_sig: String,
}

/// Gather and render. No range is the upstream to HEAD, what the next push carries.
pub fn show(a: &ShowArgs, cwd: &Path) -> Result<Printed, Refusal> {
    let repo = repo::root(cwd).map_err(|e| Refusal::repository(&e))?;
    let format = repo::format(&repo).map_err(|e| Refusal::repository(&e))?;
    let range = if a.range.is_empty() {
        let upstream = repo::upstream(&repo)
            .map_err(|e| Refusal::repository(&e))?
            .ok_or_else(Refusal::no_upstream)?;
        vec![format!("{upstream}..HEAD")]
    } else {
        a.range.clone()
    };
    let commits = range::commits(&repo, &format, "show", &range)?;
    let store = Store::open(&repo).map_err(Refusal::store)?;
    let listed = store.roots().map_err(Refusal::store)?;
    let mut notes: Vec<String> = listed.notes().collect();
    let wanted: Vec<_> = listed
        .listed
        .into_iter()
        .filter(|l| commits.iter().any(|c| c.commit == l.commit))
        .collect();
    let reads = store.read_all(&wanted).map_err(Refusal::store)?;
    let mut roots = Vec::new();
    for c in &commits {
        let Some(read) = reads.iter().find(|r| r.listed.commit == c.commit) else {
            continue;
        };
        match &read.stored {
            Ok(stored) => roots.push(Shown {
                commit: c.commit,
                name: ref_name(&c.commit),
                root_commit: read.listed.root_commit,
                root_json: String::from_utf8(
                    stored.root.canonical_bytes().map_err(Refusal::internal)?,
                )
                .map_err(Refusal::internal)?,
                root_sig: String::from_utf8_lossy(&stored.sig).into_owned(),
            }),
            Err(why) => notes.push(format!("{} is not sent: {why}", ref_name(&c.commit))),
        }
    }
    let out = ShowOut {
        v: 1,
        command: "show",
        range,
        without_ref: commits
            .iter()
            .filter(|c| !reads.iter().any(|r| r.listed.commit == c.commit))
            .map(|c| c.commit)
            .collect(),
        roots,
        notes,
    };
    let text = if a.json {
        crate::render::json(&out)?
    } else {
        render(&out)
    };
    Ok(Printed::verdict(text, true))
}

fn render(o: &ShowOut) -> String {
    let mut s = String::new();
    let _ = writeln!(
        s,
        "SHOW {}: {} roots would be sent; {} commits have no root ref of their own",
        o.range.join(" "),
        o.roots.len(),
        o.without_ref.len()
    );
    for r in &o.roots {
        let _ = writeln!(s, "ref {} {}", r.name, r.root_commit.hex());
        let _ = writeln!(s, "{}", r.root_json);
        s.push_str(&r.root_sig);
        if !r.root_sig.ends_with('\n') {
            s.push('\n');
        }
    }
    for c in &o.without_ref {
        let _ = writeln!(s, "no root ref {}", c.hex());
    }
    for note in &o.notes {
        let _ = writeln!(s, "note {note}");
    }
    s
}
