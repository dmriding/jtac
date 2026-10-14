//! `jtac check` (DESIGN.md section 11.1): preflight, gather, judge, then write what the
//! verdict allows. `--preview` stops after the verdict and touches nothing.

use std::path::Path;

use jtac_checks::Cache;
use jtac_core::Ts;
use jtac_store::Store;

use crate::cli::CheckArgs;
use crate::refuse::Refusal;
use crate::{Printed, chain, identity, judge, output, render, repo, root, state, write};

/// The rendered verdict. A root, once written, is reported even when `state.json` is not.
pub fn check(a: &CheckArgs, cwd: &Path) -> Result<Printed, Refusal> {
    if std::env::var_os(jtac_checks::NESTED_VAR).is_some_and(|v| v == "1") {
        return Err(Refusal::nested());
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
    let j = judge::judge(&repo, &loaded, head, base, now, cache.as_deref())?;
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
    write::evidence(&store, &j, cache)?;
    let root_id = match (&existing, j.verdict.pass) {
        (Some(stored), true) => {
            notes.push("HEAD already has a root; it stands, and this run wrote none".to_owned());
            Some(stored.root.id().map_err(Refusal::internal)?)
        }
        (None, true) => {
            let chain = chain::chain(&repo, &store, &covered, &head, &j.base.commit)?;
            let (root, leaves) = root::build(
                &j,
                chain,
                loaded.policy.version,
                gate,
                identity.signer.clone(),
            )
            .map_err(Refusal::internal)?;
            match write::root(&store, &identity, &root, &leaves)? {
                Some(id) => Some(id),
                None => {
                    notes.push(
                        "another run wrote HEAD's root first; it stands, and this run wrote none"
                            .to_owned(),
                    );
                    let won = store
                        .root(&head.commit)
                        .map_err(Refusal::store)?
                        .ok_or_else(|| Refusal::internal("HEAD's root ref vanished"))?;
                    Some(won.root.id().map_err(Refusal::internal)?)
                }
            }
        }
        (_, false) => None,
    };
    let out = output::output(&j, notes, false, root_id);
    let mut printed = Printed::verdict(render::render(&out, a.json)?, out.pass);
    printed.warn = state::write(&store, &out).err();
    Ok(printed)
}
