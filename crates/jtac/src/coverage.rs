//! What a pushed tip has (DESIGN.md sections 9.5, 11.3): its own root, else a root for the
//! same tree. A root counts when it names the tip's tree and its signature holds; who signed
//! is `verify`'s question. A root that exists and does not count is a note, never a pass.
//! `pre-push` and `status` both read it here, so the two never disagree.

use std::path::Path;

use jtac_core::{GitOid, Root, Sha256Digest};
use jtac_gate::Coverage;
use jtac_sign::{Namespace, SignError, Tool, Verified};
use jtac_store::Store;

use crate::refuse::Refusal;
use crate::repo::{Head, config};

/// The tip's coverage and the roots passed over.
pub struct Tip {
    /// For the decision.
    pub coverage: Coverage,
    /// The commit whose root counted, the tip's own or a tree match's, with that root's id.
    pub counted: Option<(GitOid, Sha256Digest)>,
    /// Each root that exists and does not count, with why.
    pub notes: Vec<String>,
    /// The tip's own root ref exists and did not count, so `jtac check` would keep it.
    pub own_stale: bool,
}

/// Nothing: what a deletion has.
pub fn none() -> Tip {
    Tip {
        coverage: Coverage::Absent,
        counted: None,
        notes: Vec::new(),
        own_stale: false,
    }
}

/// ssh-keygen as git config names it.
pub fn verifier(repo: &Path) -> Result<Tool, Refusal> {
    let program = config(repo, "gpg.ssh.program").map_err(|e| Refusal::repository(&e))?;
    jtac_sign::verifier(program.as_deref()).map_err(|e| Refusal::verifier(&e))
}

/// The tip's own root, else the first root in ref order that judged the same tree.
pub fn tip(store: &Store, tool: &Tool, tip: &Head) -> Result<Tip, Refusal> {
    let mut notes = Vec::new();
    let mut found = counted(store, tool, tip, &tip.commit, &mut notes)?.map(|r| (tip.commit, r));
    let own_stale = found.is_none() && !notes.is_empty();
    let roots = store.roots().map_err(Refusal::store)?;
    notes.extend(roots.notes());
    if found.is_none() {
        for listed in roots.listed {
            if listed.commit == tip.commit {
                continue;
            }
            if let Some(root) = counted(store, tool, tip, &listed.commit, &mut notes)? {
                found = Some((listed.commit, root));
                break;
            }
        }
    }
    let Some((commit, root)) = found else {
        return Ok(Tip {
            coverage: Coverage::Absent,
            counted: None,
            notes,
            own_stale,
        });
    };
    let coverage = if commit == tip.commit {
        Coverage::Root(root.state)
    } else {
        Coverage::ByTree {
            commit: commit.hex(),
        }
    };
    Ok(Tip {
        coverage,
        counted: Some((commit, root.id().map_err(Refusal::internal)?)),
        notes,
        own_stale,
    })
}

/// `commit`'s root when it names the tip's tree and its signature holds. A tree that differs
/// is a note only for the tip's own root; for any other it is just not a match.
fn counted(
    store: &Store,
    tool: &Tool,
    tip: &Head,
    commit: &GitOid,
    notes: &mut Vec<String>,
) -> Result<Option<Root>, Refusal> {
    let mut not =
        |why: String| notes.push(format!("{}'s root is not counted: {why}", commit.hex()));
    let stored = match store.root(commit) {
        Ok(Some(stored)) => stored,
        Ok(None) => return Ok(None),
        Err(e) => {
            not(e.to_string());
            return Ok(None);
        }
    };
    let root = stored.root;
    if root.tree != tip.tree {
        if *commit == tip.commit {
            not(format!("it names tree {}, not {}", root.tree, tip.tree));
        }
        return Ok(None);
    }
    let bytes = root.canonical_bytes().map_err(Refusal::internal)?;
    let sig = String::from_utf8_lossy(&stored.sig);
    let verified = jtac_sign::verify(
        tool,
        None,
        &root.signer,
        Namespace::Root,
        root.ts,
        &bytes,
        &sig,
    )
    .map_err(|e| Refusal::verifier(&e))?;
    match verified {
        Verified::Trusted | Verified::Unknown { .. } => Ok(Some(root)),
        Verified::Invalid { what } => {
            not(format!("its signature does not hold: {what}"));
            Ok(None)
        }
    }
}

impl Refusal {
    /// ssh-keygen could not run to check a root's signature.
    pub fn verifier(err: &SignError) -> Self {
        Self {
            kind: "negotiation",
            what: format!("cannot check a root's signature: {err}."),
            why: "A root counts only when its signature holds; a signature nobody could check \
                  must never read as one that held, so the push is refused rather than passed."
                .to_owned(),
            how: "Install OpenSSH's ssh-keygen, or name it with `git config gpg.ssh.program \
                  <path to ssh-keygen>`, then run the command again."
                .to_owned(),
        }
    }
}
