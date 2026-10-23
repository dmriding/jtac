//! `jtac ci` (DESIGN.md section 15): the pull request gate. One verdict on HEAD from its
//! merge base with `--base`, trust from that rev's tree, the chain's trusted waivers offered,
//! then HEAD's root verified as a document and compared with this gate's own run of its diff.
//! It signs nothing and writes no root; the exit code is the answer. Evidence nodes go to the
//! store so an uploaded store replays.

mod head;
mod levers;
mod out;

use std::path::Path;

use jtac_checks::Cache;
use jtac_checks::diff::Adoption;
use jtac_core::Ts;
use jtac_store::Store;

use crate::cli::CiArgs;
use crate::refuse::Refusal;
use crate::trust::Signers;
use crate::{Printed, coverage, judge, output, policy, repo, write};

/// The rendered verdict; exit 1 on CI's own block, a head root that is invalid, lied or
/// disagrees, or with `--require-local-root` one missing or unverifiable.
pub fn ci(a: &CiArgs, cwd: &Path) -> Result<Printed, Refusal> {
    if std::env::var_os(jtac_checks::NESTED_VAR).is_some_and(|v| v == "1") {
        return Err(Refusal::nested());
    }
    let repo = repo::root(cwd).map_err(|e| Refusal::repository(&e))?;
    let format = repo::format(&repo).map_err(|e| Refusal::repository(&e))?;
    let head = repo::head(&repo).map_err(|e| Refusal::no_head(&e))?;
    if let Some(why) = repo::dirty(&repo).map_err(|e| Refusal::repository(&e))? {
        return Err(Refusal::dirty(&why));
    }
    if let Some(path) = repo::grafted(&repo).map_err(|e| Refusal::repository(&e))? {
        return Err(Refusal::grafted(&path));
    }
    if repo::shallow(&repo).map_err(|e| Refusal::repository(&e))? {
        return Err(Refusal::shallow());
    }
    let base_tip = repo::peel(&repo, &format, &a.base).map_err(|_| Refusal::rev("ci", &a.base))?;
    let base =
        repo::base(&repo, &head, Some(&a.base), None).map_err(|e| Refusal::base(&a.base, &e))?;
    let base_trust = Signers::at(&repo, &base_tip, &a.base)?;
    let adoption = adoption_of(&base_trust);
    let signers = match &a.signers {
        Some(path) => Signers::File(path.clone()),
        None => base_trust,
    };
    let head_policy = policy::load_if_present(&repo, &head)?;
    let base_policy = match touches_jtac(&repo, &base.commit, &head.commit.hex())? {
        true => policy::load_if_present(&repo, &base_tip)?,
        false => None,
    };
    let (loaded, policy_from) = match (&base_policy, &head_policy) {
        (Some(l), _) => (l, "base"),
        (None, Some(l)) => (l, "head"),
        (None, None) => return Err(Refusal::no_policy()),
    };
    let store = Store::open(&repo).map_err(Refusal::store)?;
    let tool = coverage::verifier(&repo)?;
    let chain = levers::gather(&repo, &store, &tool, &signers, &format, &head, &base.commit)?;
    let now = Ts::from_timestamp(jiff::Timestamp::now()).map_err(Refusal::internal)?;
    let mut cache = store.cache();
    let offered = judge::Offered {
        waivers: chain.trusted.clone(),
        waivable: chain.commits.clone(),
        adoption,
    };
    let j = judge::judge(
        &repo,
        loaded,
        head,
        base,
        now,
        Some(&cache as &dyn Cache),
        &offered,
    )?;
    let bytes = jtac_core::canonical_bytes(&loaded.policy).map_err(Refusal::internal)?;
    write::evidence(&store, &bytes, &j, &mut cache)?;
    let site = head::Site {
        repo: &repo,
        format: &format,
        head,
        head_policy: head_policy.as_ref(),
        ci: &j,
    };
    let head_root = head::compare_head(&site, &chain.ledger)?;
    let mut notes = chain.notes;
    notes.extend(chain.ledger.notes.iter().cloned());
    let withheld = out::withheld(&j, &chain.withheld);
    let mut o = out::CiOut {
        v: 1,
        command: "ci",
        base_rev: a.base.clone(),
        signers: signers.describe(),
        adoption: adoption == Adoption::Trust,
        policy_from,
        waivable: chain.commits.len() as u64,
        verdict: output::output(&j, notes, false, None),
        withheld_waivers: withheld,
        head_root,
        require_local_root: a.require_local_root,
        reasons: Vec::new(),
        pass: false,
    };
    o.decide();
    let text = if a.json {
        crate::render::json(&o)?
    } else {
        out::render(&o)
    };
    Ok(Printed::verdict(text, o.pass))
}

/// Decision 45's CI fact: a base tree with no signers file had nothing in force, whatever
/// trust `--signers` supplies.
pub fn adoption_of(base_trust: &Signers) -> Adoption {
    match base_trust {
        Signers::None { .. } => Adoption::Trust,
        _ => Adoption::None,
    }
}

/// Whether the diff from `base` to `tip` touches `.jtac/**`: then the base branch's policy
/// judges it, so no run clears the policy it brings (DESIGN.md section 15).
pub fn touches_jtac(repo: &Path, base: &str, tip: &str) -> Result<bool, Refusal> {
    let args = [
        "diff",
        "--quiet",
        "--end-of-options",
        base,
        tip,
        "--",
        ".jtac",
    ];
    Ok(repo::probe(repo, &args, 1)
        .map_err(|e| Refusal::repository(&e))?
        .is_none())
}
