//! `jtac check` (DESIGN.md section 11.1): preflight, gather, judge, then write what the
//! verdict allows. `--preview` stops after the verdict and touches nothing.

use std::collections::BTreeSet;
use std::path::Path;

use jtac_checks::Cache;
use jtac_core::{Body, FromHead, GateRef, Root, Scope, Sha256Digest, Ts};
use jtac_policy::Checks;
use jtac_sign::Identity;
use jtac_store::{Store, StoredRoot};

use crate::chain::Chain;
use crate::cli::CheckArgs;
use crate::judge::Judged;
use crate::refuse::Refusal;
use crate::survey::state_name;
use crate::{
    Printed, chain, identity, judge, lever, merge, output, render, repo, root, rootpush, state,
    treepolicy, write,
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
    let landed = repo::landed(&repo, &head).map_err(|e| Refusal::repository(&e))?;
    let base = repo::base(
        &repo,
        &head,
        a.base.as_deref(),
        nearest.as_deref(),
        landed.as_ref(),
    )
    .map_err(|e| match &a.base {
        Some(rev) => Refusal::base(rev, &e),
        None => Refusal::repository(&e),
    })?;
    if a.base.is_none() && repo::default_unnamed(&repo).map_err(|e| Refusal::repository(&e))? {
        notes.push(
            "this clone does not name origin's default branch (`origin/HEAD` is missing or \
             names a branch that is gone), so the base ignores what that branch already holds; \
             run `git remote set-head origin --auto`"
                .to_owned(),
        );
    }
    let now = Ts::from_timestamp(jiff::Timestamp::now()).map_err(Refusal::internal)?;
    let mut file_cache = store.cache();
    let cache: Option<&mut dyn Cache> = signer.as_ref().map(|_| &mut file_cache as &mut dyn Cache);
    let offered = judge::Offered {
        waivers: lever::offered(&repo, &store, &mut notes)?,
        waivable: Vec::new(),
        adoption: judge::adoption(&repo, &head)?,
        checks: Checks::Run,
        observed: Vec::new(),
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
        repo: &repo,
        store: &store,
        identity: &identity,
        gate,
        policy_version: loaded.policy.version,
        existing: existing.as_ref(),
        from: None,
    };
    let chain = || chain::chain(&repo, &store, &covered, &head, &j.base.commit);
    let root_id = match landed.filter(|l| withheld(l, &j, existing.is_some())) {
        Some(l) => {
            notes.push(not_this_clones(&l.shown, &head.commit.hex()));
            None
        }
        None => site.commit(&j, chain, &mut notes)?.map(|w| w.id),
    };
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

/// Whether a passing run leaves no root (DESIGN.md section 10.3, decision 69): the remote's
/// default branch already holds HEAD, HEAD has none, and no commit waiver names it. Such a
/// root would vouch for a commit that left another machine and stop the merge job, which
/// keeps only its own; the waiver is the bypass merge's recovery and still gets its root
/// (`a_commit_the_default_branch_holds_gets_no_root_unless_a_waiver_names_it`).
fn withheld(landed: &repo::Landed, j: &Judged, rooted: bool) -> bool {
    let waived = j.levers.iter().any(
        |l| matches!(&l.node.body, Body::Lever(b) if matches!(b.scope, Some(Scope::Commit { .. }))),
    );
    landed.commit == j.head.commit.hex() && j.verdict.pass && !rooted && !waived
}

/// The note a withheld root leaves: what, why, and the three ways on.
fn not_this_clones(shown: &str, hex: &str) -> String {
    let short = &hex[..12];
    format!(
        "no root was written: {shown} already holds {short}, so this run judged a commit that \
         left another machine, and a root from here would stand where CI's merge root belongs. \
         To push a branch cut from it, commit first; to wait for CI's root, `git fetch origin \
         '+refs/jtac/*:refs/jtac/*'`; if it reached {shown} around the gate, say why with `jtac \
         waive {short} --reason \"<why>\"` (add `--ci` where CI should have covered it), then \
         `jtac check` again"
    )
}

/// What a root is written with, for `check` and the merge job alike.
pub struct Site<'a> {
    /// The repository.
    pub repo: &'a Path,
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
                self.own_reader(&root)?;
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

    /// The writer signs no root its own reader refuses (decision 57): a root without
    /// `from` lists as `unread` the `ci` checks of the policy in the tree it names, as
    /// that reader reads the tree. A drift between the gate's load and the reader's is a
    /// refusal here, never an invalid root on the remote.
    fn own_reader(&self, root: &Root) -> Result<(), Refusal> {
        if root.from.is_some() {
            return Ok(());
        }
        let read = treepolicy::read(self.repo, &BTreeSet::from([root.tree]))?;
        match read[&root.tree].refuses(&root.unread) {
            None if read[&root.tree].ci_checks().is_some() => Ok(()),
            why => Err(Refusal::internal(format!(
                "this gate built a root its own reader would not count: {}",
                why.unwrap_or_else(|| "the policy in HEAD's tree does not read back".into())
            ))),
        }
    }
}
