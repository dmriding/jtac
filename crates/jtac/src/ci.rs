//! `jtac ci` (DESIGN.md section 15): the pull request gate. One verdict on HEAD from its
//! merge base with `--base`, trust from that rev's tree, the chain's trusted waivers offered,
//! then HEAD's root verified as a document and compared with this gate's own run of its diff.
//! It signs nothing and writes no root; the exit code is the answer. Evidence nodes go to the
//! store so an uploaded store replays.

pub(crate) mod head;
pub(crate) mod levers;
pub(crate) mod out;

use std::path::Path;

use jtac_checks::Cache;
use jtac_checks::diff::Adoption;
use jtac_core::Ts;
use jtac_store::Store;

use crate::cli::CiArgs;
use crate::policy::Loaded;
use crate::refuse::Refusal;
use crate::repo::Head;
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
    let mut notes = Vec::new();
    let policies = policies(&repo, &head, &base_tip, &base.commit, &mut notes)?;
    let (loaded, policy_from) = policies.judging();
    let head_policy = policies.head.as_ref();
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
        cwd: &repo,
        format: &format,
        head,
        head_policy,
        ci: &j,
        cache: &cache,
        keyed: false,
    };
    let head_root = head::compare_head(&site, &chain.ledger)?;
    notes.extend(chain.notes);
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

/// The policies a CI run reads (DESIGN.md section 15): the base branch's judges when the
/// diff touches `.jtac/**`, or when HEAD carries no readable policy and the base holds one;
/// else HEAD's. Both none is a refusal, since the gate could not answer.
pub struct Policies {
    /// HEAD's own, `None` when HEAD carries none or it does not read beside a base policy.
    pub head: Option<Loaded>,
    /// The base branch's, read only when it judges.
    pub base: Option<Loaded>,
}

impl Policies {
    /// The policy the verdict runs under and where it came from, `head` or `base`.
    pub fn judging(&self) -> (&Loaded, &'static str) {
        match (&self.base, &self.head) {
            (Some(l), _) => (l, "base"),
            (None, Some(l)) => (l, "head"),
            (None, None) => unreachable!("`policies` refused both none"),
        }
    }
}

/// Read both sides for a run whose diff is `diff_base..head` and whose base branch tip is
/// `base_tip`; a HEAD policy that does not read is a note when the base's judges.
pub fn policies(
    repo: &Path,
    head: &Head,
    base_tip: &Head,
    diff_base: &str,
    notes: &mut Vec<String>,
) -> Result<Policies, Refusal> {
    let head_policy = policy::load_if_present(repo, head);
    let base = match (
        &head_policy,
        touches_jtac(repo, diff_base, &head.commit.hex())?,
    ) {
        (Ok(Some(_)), false) => None,
        _ => policy::load_if_present(repo, base_tip)?,
    };
    let head = match (head_policy, &base) {
        (Ok(p), _) => p,
        (Err(e), Some(_)) => {
            notes.push(format!(
                "HEAD's policy does not read ({}); the base branch's judges",
                e.what
            ));
            None
        }
        (Err(e), None) => return Err(e),
    };
    if base.is_none() && head.is_none() {
        return Err(Refusal::no_policy());
    }
    Ok(Policies { head, base })
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
