//! `jtac show` (DESIGN.md sections 7.1, 11.9 and 19): the root refs a push of the range would
//! send, chosen as the hook chooses them (`rootpush`): every commit in the range with a ref,
//! whatever the ref holds, plus the root that covered the tip by tree when it has none of
//! its own. Each with the two blobs its commit carries. Nothing is judged here; `verify` does.
//! A failure record on a commit of the range is printed apart: the merge job sends it, no
//! push of the range does (section 15.2).

use std::fmt::Write;
use std::path::Path;

use jtac_core::GitOid;
use jtac_store::{Read, Space, Store};
use serde::Serialize;

use crate::Printed;
use crate::cli::ShowArgs;
use crate::refuse::Refusal;
use crate::repo::Head;
use crate::{coverage, range, repo};

/// What `jtac show` prints.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ShowOut {
    /// Output schema version.
    pub v: u32,
    /// The command.
    pub command: &'static str,
    /// The range as listed.
    pub range: Vec<String>,
    /// The refs a push would send, range order, the tip's tree match last.
    pub roots: Vec<Shown>,
    /// The failure records the range's commits hold, range order; no push sends one.
    pub records: Vec<Shown>,
    /// Commits in the range with no root ref of their own; a later root may still cover them.
    pub without_ref: Vec<GitOid>,
    /// Skipped names and what the tip's coverage passed over.
    pub notes: Vec<String>,
}

/// One root ref as the push sends it, or one failure record ref as this clone holds it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Shown {
    /// The commit the document is for.
    pub commit: GitOid,
    /// The ref.
    pub name: String,
    /// The root commit the ref points at.
    pub root_commit: GitOid,
    /// `root.json`, the canonical bytes; empty when the ref does not hold its document.
    pub root_json: String,
    /// `root.sig`, the armored signature; empty when the ref does not hold its document.
    pub root_sig: String,
    /// Why the ref is sent as a tree match, or why its content is not a root.
    pub note: Option<String>,
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
    let reads = store.read_all(&listed).map_err(Refusal::store)?;
    let held = store.records().map_err(Refusal::store)?;
    notes.extend(held.notes());
    let held = store.read_all(&held).map_err(Refusal::store)?;
    let shown = |commit, note| document(Space::Roots, &reads, commit, note);
    let mut roots = Vec::new();
    let mut records = Vec::new();
    let mut without_ref = Vec::new();
    for c in &commits {
        match shown(c.commit, None)? {
            Some(s) => roots.push(s),
            None => without_ref.push(c.commit),
        }
        records.extend(document(Space::Failed, &held, c.commit, None)?);
    }
    if let Some(tip) = commits.first() {
        let tool = coverage::verifier(&repo)?;
        let head = Head {
            commit: tip.commit,
            tree: tip.tree,
        };
        let cover = coverage::tip(&store, &tool, &head, None)?;
        notes.extend(cover.notes);
        if let Some((by, _)) = cover.counted
            && by != tip.commit
            && !roots.iter().any(|s| s.commit == by)
        {
            let note = format!(
                "sent for the tip's tree: it judged what {} holds",
                tip.commit.hex()
            );
            roots.extend(shown(by, Some(note))?);
        }
    }
    let out = ShowOut {
        v: 1,
        command: "show",
        range,
        roots,
        records,
        without_ref,
        notes,
    };
    let text = if a.json {
        crate::render::json(&out)?
    } else {
        render(&out)
    };
    Ok(Printed::verdict(text, true))
}

/// The ref `space` holds for `commit` among `reads`, with its two blobs when it holds its
/// document and the reason when it does not.
fn document(
    space: Space,
    reads: &[Read],
    commit: GitOid,
    note: Option<String>,
) -> Result<Option<Shown>, Refusal> {
    let Some(read) = reads.iter().find(|r| r.listed.commit == commit) else {
        return Ok(None);
    };
    let (root_json, root_sig, note) = match (&read.stored, space) {
        (Ok(stored), _) => (
            String::from_utf8(stored.root.canonical_bytes().map_err(Refusal::internal)?)
                .map_err(Refusal::internal)?,
            String::from_utf8_lossy(&stored.sig).into_owned(),
            note,
        ),
        (Err(why), Space::Roots) => (
            String::new(),
            String::new(),
            Some(format!("sent as it is; it does not hold a root: {why}")),
        ),
        (Err(why), Space::Failed) => (
            String::new(),
            String::new(),
            Some(format!("it does not hold a failure record: {why}")),
        ),
    };
    Ok(Some(Shown {
        commit,
        name: space.ref_name(&commit),
        root_commit: read.listed.root_commit,
        root_json,
        root_sig,
        note,
    }))
}

fn render(o: &ShowOut) -> String {
    let mut s = String::new();
    let _ = write!(
        s,
        "SHOW {}: {} refs would be sent; {} commits have no root ref of their own",
        o.range.join(" "),
        o.roots.len(),
        o.without_ref.len()
    );
    let _ = match o.records.len() {
        0 => writeln!(s),
        held => writeln!(s, "; {held} failure records are held and not sent"),
    };
    let rows = o.records.iter().map(|r| ("record", r));
    for (word, r) in rows.chain(o.roots.iter().map(|r| ("ref", r))) {
        let _ = writeln!(s, "{word} {} {}", r.name, r.root_commit.hex());
        if let Some(note) = &r.note {
            let _ = writeln!(s, "note {note}");
        }
        if !r.root_json.is_empty() {
            let _ = writeln!(s, "{}", r.root_json);
            s.push_str(&r.root_sig);
            if !r.root_sig.ends_with('\n') {
                s.push('\n');
            }
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
