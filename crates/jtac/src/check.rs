//! `jtac check` (DESIGN.md section 11.1): preflight, gather, judge, then write what the
//! verdict allows. `--preview` stops after the verdict and touches nothing.

use std::path::Path;

use jtac_checks::Cache;
use jtac_core::{FromHead, GateRef, Sha256Digest, Ts};
use jtac_sign::Identity;
use jtac_store::{Store, StoredRoot};

use crate::chain::Chain;
use crate::cli::CheckArgs;
use crate::judge::Judged;
use crate::refuse::Refusal;
use crate::survey::state_name;
use crate::{
    Printed, chain, identity, judge, lever, merge, output, render, repo, root, rootpush, state,
    write,
};

/// The rendered verdict. A root, once written, is reported even when `state.json` is not.
pub fn check(a: &CheckArgs, cwd: &Path) -> Result<Printed, Refusal> {
    if std::env::var_os(jtac_checks::NESTED_VAR).is_some_and(|v| v == "1") {
        return Err(Refusal::nested());
    }
    if a.from.is_some() {
        return merge::check(a, cwd);
    }
    let repo = repo::root(cwd).map_err(|e| Refusal::repository(&e))?;
    let head = repo::head(&repo).map_err(|e| Refusal::no_head(&e))?;
    let loaded = crate::policy::load(&repo, &head)?;
    let dirty = repo::dirty(&repo).map_err(|e| Refusal::repository(&e))?;
    if let (Some(why), false) = (&dirty, a.preview) {
        return Err(Refusal::dirty(why));
    }
    if let Some(path) = repo::grafted(&repo).map_err(|e| Refusal::repository(&e))? {
        return Err(Refusal::grafted(&path));
    }
    if !a.preview && repo::shallow(&repo).map_err(|e| Refusal::repository(&e))? {
        return Err(Refusal::shallow());
    }
    let signer = if a.preview {
        None
    } else {
        Some((identity::identity(&repo)?, identity::gate_ref()?))
    };
    let store = Store::open(&repo).map_err(Refusal::store)?;
    let existing = store.root(&head.commit).map_err(Refusal::store)?;
    let roots = store.roots().map_err(Refusal::store)?;
    let mut notes: Vec<String> = roots.notes().collect();
    let covered = chain::covered(&repo, &roots, &head)?;
    let nearest = chain::nearest(&repo, &covered, &head)?;
    let base =
        repo::base(&repo, &head, a.base.as_deref(), nearest.as_deref()).map_err(|e| {
            match &a.base {
                Some(rev) => Refusal::base(rev, &e),
                None => Refusal::repository(&e),
            }
        })?;
    let now = Ts::from_timestamp(jiff::Timestamp::now()).map_err(Refusal::internal)?;
    let mut file_cache = store.cache();
    let cache: Option<&mut dyn Cache> = signer.as_ref().map(|_| &mut file_cache as &mut dyn Cache);
    let offered = judge::Offered {
        waivers: lever::offered(&repo, &store, &mut notes)?,
        waivable: Vec::new(),
        adoption: judge::adoption(&repo, &head)?,
    };
    let j = judge::judge(&repo, &loaded, head, base, now, cache.as_deref(), &offered)?;
    if let Some(why) = dirty {
        notes.push(format!(
            "the work tree is not HEAD's tree ({why}): the checks ran on it, the diff-integrity \
             rules judged HEAD; commit before `jtac check`"
        ));
    }
    if loaded.differs_on_disk {
        notes.push("the policy on disk differs from HEAD's; the verdict used HEAD's".to_owned());
    }
    let Some((identity, gate)) = signer else {
        let out = output::output(&j, notes, true, None);
        return Ok(Printed::verdict(render::render(&out, a.json)?, out.pass));
    };
    let Some(cache) = cache else {
        unreachable!("a signer and a cache come together")
    };
    if repo::head(&repo).map_err(|e| Refusal::no_head(&e))? != head {
        return Err(Refusal::moved(None));
    }
    if let Some(why) = repo::dirty(&repo).map_err(|e| Refusal::repository(&e))? {
        return Err(Refusal::moved(Some(&why)));
    }
    let policy = jtac_core::canonical_bytes(&loaded.policy).map_err(Refusal::internal)?;
    write::evidence(&store, &policy, &j, cache)?;
    let site = Site {
        store: &store,
        identity: &identity,
        gate,
        policy_version: loaded.policy.version,
        existing: existing.as_ref(),
        from: None,
    };
    let chain = || chain::chain(&repo, &store, &covered, &head, &j.base.commit);
    let root_id = site.commit(&j, chain, &mut notes)?.map(|w| w.id);
    let sent = match (&a.push, root_id) {
        (Some(remote), Some(_)) => Some(rootpush::send(&store, remote, &[head.commit])),
        (Some(_), None) => {
            notes.push("nothing was pushed: this run left no root to send".to_owned());
            None
        }
        (None, _) => None,
    };
    if let (Some(remote), Some(s)) = (&a.push, &sent) {
        notes.extend(s.held.iter().map(|r| format!("{remote} holds {r}")));
    }
    let out = output::output(&j, notes, false, root_id);
    let mut printed = Printed::verdict(render::render(&out, a.json)?, out.pass);
    printed.warn = state::write(&store, &out).err();
    printed.note = sent.map(|s| s.text).filter(|t| !t.is_empty());
    Ok(printed)
}

/// What a root is written with, for `check` and the merge job alike.
pub struct Site<'a> {
    /// The store.
    pub store: &'a Store,
    /// Who signs.
    pub identity: &'a Identity,
    /// This binary.
    pub gate: GateRef,
    /// `policy.version` of the policy that judged.
    pub policy_version: u32,
    /// HEAD's root before this run, which stands (decision 33).
    pub existing: Option<&'a StoredRoot>,
    /// The merge job's head (DESIGN.md section 15); `None` for a local root.
    pub from: Option<FromHead>,
}

/// The root this run left HEAD with and whether this run wrote it.
pub struct Written {
    /// The root id.
    pub id: Sha256Digest,
    /// `false` when a root already stood, this run's or another's.
    pub fresh: bool,
}

impl Site<'_> {
    /// Write the root on a pass when HEAD has none; `chain` is computed only then. A root
    /// that stands, HEAD's own or one another run created first, is reported in `notes`.
    pub fn commit(
        &self,
        j: &Judged,
        chain: impl FnOnce() -> Result<Chain, Refusal>,
        notes: &mut Vec<String>,
    ) -> Result<Option<Written>, Refusal> {
        let found = |id, fresh| Ok(Some(Written { id, fresh }));
        match (self.existing, j.verdict.pass) {
            (Some(stored), true) => {
                notes.push(format!(
                    "HEAD already has a root; it stands, and this run wrote none: it records {} \
                     with {} waivers, this run judged {} with {}",
                    state_name(Some(stored.root.state)),
                    stored.root.levers.len(),
                    state_name(Some(j.verdict.state)),
                    j.levers.len()
                ));
                found(stored.root.id().map_err(Refusal::internal)?, false)
            }
            (None, true) => {
                let (root, leaves) = root::build(
                    j,
                    chain()?,
                    self.policy_version,
                    self.gate.clone(),
                    self.identity.signer.clone(),
                    self.from.clone(),
                )
                .map_err(Refusal::internal)?;
                match write::root(self.store, self.identity, &root, &leaves)? {
                    Some(id) => found(id, true),
                    None => {
                        notes.push(
                            "another run wrote HEAD's root first; it stands, and this run wrote \
                             none"
                                .to_owned(),
                        );
                        let won = self
                            .store
                            .root(&j.head.commit)
                            .map_err(Refusal::store)?
                            .ok_or_else(|| Refusal::internal("HEAD's root ref vanished"))?;
                        found(won.root.id().map_err(Refusal::internal)?, false)
                    }
                }
            }
            (_, false) => Ok(None),
        }
    }
}
