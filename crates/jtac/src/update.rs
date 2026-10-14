//! The facts of one ref update for `jtac_gate::decide` (DESIGN.md section 11.3): the fence
//! from the tip's policy joined with the remote's old tip's, the old oid against the tip, and
//! what the tip has. `pre-push` gathers them per line git hands it, `status` for HEAD against
//! its upstream.

use std::path::Path;

use jtac_checks::git::{GitError, git};
use jtac_core::GitOid;
use jtac_gate::{History, RefUpdate};
use jtac_policy::FenceMode;
use jtac_sign::Tool;
use jtac_store::Store;

use crate::coverage::{self, Tip};
use crate::policy::{self, Loaded};
use crate::refuse::Refusal;
use crate::repo::{Head, RepoError, probe};

/// The update and the coverage it was decided on.
pub struct Gathered {
    /// For the decision.
    pub update: RefUpdate,
    /// The roots behind `update.tip`.
    pub tip: Tip,
}

/// `tip` pushed to `remote_ref` over `old`, the remote's oid when it has the ref; no tip is
/// a deletion, whose only policy is the old tip's.
pub fn gather(
    repo: &Path,
    store: &Store,
    tool: &Tool,
    format: &str,
    tip: Option<&Head>,
    remote_ref: &str,
    old: Option<&GitOid>,
) -> Result<Gathered, Refusal> {
    let (mut protected, mut deny_force) = match tip {
        Some(tip) => fence(&policy::load(repo, tip)?, remote_ref),
        None => (false, false),
    };
    let history = history(repo, old, tip.map(|t| &t.commit))?;
    if let (Some(old), History::FastForward | History::Rewinds) = (old, history) {
        let was =
            crate::repo::peel(repo, format, &old.hex()).map_err(|e| Refusal::repository(&e))?;
        if let Some(loaded) = policy::load_if_present(repo, &was)? {
            let (p, d) = fence(&loaded, remote_ref);
            protected |= p;
            deny_force |= d;
        }
    }
    let covered = match tip {
        Some(tip) => coverage::tip(store, tool, tip)?,
        None => coverage::none(),
    };
    Ok(Gathered {
        update: RefUpdate {
            remote_ref: remote_ref.to_owned(),
            delete: tip.is_none(),
            protected,
            deny_force,
            history,
            tip: covered.coverage.clone(),
        },
        tip: covered,
    })
}

/// Whether `remote_ref` is a protected branch under this policy, and whether a rewind of one
/// is denied. The remote's old tip gets a say so a tip cannot carry its own exemption
/// (`an_orphan_whose_policy_allows_the_rewind_is_still_fenced_by_the_branch`).
fn fence(loaded: &Loaded, remote_ref: &str) -> (bool, bool) {
    let protected = remote_ref
        .strip_prefix("refs/heads/")
        .is_some_and(|branch| loaded.matchers.protected_branches.is_match(branch));
    (
        protected,
        loaded.policy.fences.force_push_protected == FenceMode::Deny,
    )
}

/// No old oid creates the ref; one this clone lacks cannot be judged, so it is `Unknown`; a
/// deletion of a held tip has no new side and reads as a fast-forward, which the decision
/// never consults for one.
fn history(repo: &Path, old: Option<&GitOid>, new: Option<&GitOid>) -> Result<History, Refusal> {
    let Some(old) = old else {
        return Ok(History::Creates);
    };
    let old = old.hex();
    match git(repo, &["cat-file", "-e", &format!("{old}^{{commit}}")]) {
        Ok(_) => {}
        Err(GitError::Failed { .. }) => return Ok(History::Unknown),
        Err(e) => return Err(Refusal::repository(&RepoError::Git(e))),
    }
    let Some(new) = new else {
        return Ok(History::FastForward);
    };
    let ancestor = probe(repo, &["merge-base", "--is-ancestor", &old, &new.hex()], 1)
        .map_err(|e| Refusal::repository(&e))?;
    Ok(match ancestor {
        Some(_) => History::FastForward,
        None => History::Rewinds,
    })
}
