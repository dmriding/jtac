//! HEAD's root compared with this gate's own run of its diff (DESIGN.md section 15,
//! Verification). The ledger judged it as a document; here the gate version and the hash of
//! the policy it should have judged under say whether a run is owed, and the run is `replay`'s
//! shape with fresh rules: the root's own base and levers, its `ts` as `now`, CI's check nodes
//! where the policy and the tree are CI's, and the checks run on the head's tree where
//! either is not, through the cache, so a merge whose tree is the head's hits.

use std::path::Path;

use jtac_checks::diff::git::changes;
use jtac_checks::diff::{Adoption, report};
use jtac_checks::{CAPTURE_CAP, Cache, Checked, Context, run_cached, specs};
use jtac_core::{
    Body, Compared, Counts, Facts, FindingBody, FromHead, GitOid, Node, Root, RootVersion, Run,
    Sha256Digest, Signer, State, compare, counts, gate,
};
use jtac_gate::Trust;
use jtac_policy::{INTEGRITY_CHECK_ID, facts};
use serde::Serialize;

use crate::judge::{self, Judged};
use crate::ledger::Ledger;
use crate::policy::{self, Loaded};
use crate::refuse::Refusal;
use crate::repo::{self, Head};
use crate::trust::Signers;
use crate::{ci, range};

/// What the comparison reads from the run around it.
pub struct Site<'a> {
    /// The repository.
    pub repo: &'a Path,
    /// Where the head's tree is checked out: the repository for `jtac ci`, a detached
    /// worktree for a merge job whose tip's tree is not the head's.
    pub cwd: &'a Path,
    /// `sha1` or `sha256`.
    pub format: &'a str,
    /// HEAD.
    pub head: Head,
    /// HEAD's policy, the one a root without `from` should have judged under; `None` when HEAD
    /// carries no policy file.
    pub head_policy: Option<&'a Loaded>,
    /// CI's own run, whose check nodes judged the tip's tree.
    pub ci: &'a Judged,
    /// The store's cache: a check already run on the head's tree under its policy is a hit.
    pub cache: &'a dyn Cache,
    /// The merge job, which holds the CI key: a root with `from` whose declared base names
    /// a policy that is neither the head's own nor the branch's is unverifiable there, since
    /// its signer chose that base and its checks would run beside the key (decision 47).
    pub keyed: bool,
}

/// What `jtac ci` says about HEAD's root.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct HeadRoot {
    /// The root id, when it could be read.
    pub root: Option<Sha256Digest>,
    /// Who signed.
    pub signer: Option<Signer>,
    /// `trusted`, `unknown`, or `unchecked` for a root that was invalid before its signature.
    pub trust: &'static str,
    /// `gate.version` of the root.
    pub gate_version: String,
    /// The comparison's answer.
    pub compared: Compared,
    /// Why it is invalid or unverifiable.
    pub why: Option<String>,
    /// What the root records, when it could be read.
    pub recorded: Option<Side>,
    /// What this gate's run from the root's base reached, when one was owed.
    pub run: Option<Side>,
}

/// One side of the comparison.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Side {
    /// The state.
    pub state: State,
    /// The counts.
    pub counts: Counts,
}

/// HEAD's root judged and compared, `None` when HEAD has none.
pub fn compare_head(site: &Site, ledger: &Ledger) -> Result<Option<HeadRoot>, Refusal> {
    let Some(i) = ledger
        .roots
        .iter()
        .position(|r| r.commit == site.head.commit)
    else {
        return Ok(None);
    };
    let fact = &ledger.roots[i];
    let mut out = HeadRoot {
        root: fact.id,
        signer: fact.signer.clone(),
        trust: match &fact.trust {
            Trust::Trusted => "trusted",
            Trust::Unknown { .. } => "unknown",
            Trust::Unchecked => "unchecked",
        },
        gate_version: fact.gate_version.clone(),
        compared: Compared::Invalid,
        why: fact.invalid.clone(),
        recorded: None,
        run: None,
    };
    let Some(stored) = ledger.stored[i].as_ref().filter(|_| fact.invalid.is_none()) else {
        return Ok(Some(out));
    };
    let root = &stored.root;
    out.recorded = Some(Side {
        state: root.state,
        counts: root.counts.clone(),
    });
    let gate_version = env!("CARGO_PKG_VERSION").to_owned();
    if root.v == RootVersion::V1 {
        out.compared = Compared::Unverifiable;
        out.why =
            Some("a version 1 root is from another gate version and is never compared".into());
        return Ok(Some(out));
    }
    if root.gate.version != gate_version {
        out.compared = Compared::Unverifiable;
        out.why = Some(format!(
            "it is from gate version {}, and this is {gate_version}",
            root.gate.version
        ));
        return Ok(Some(out));
    }
    let base = root
        .base
        .map_or_else(|| site.head.commit.hex(), |b| b.hex());
    let declared = match root.from.as_ref().map(|_| declared(site, &base)) {
        None => None,
        Some(Ok(d)) => Some(d),
        Some(Err(e)) => {
            out.compared = Compared::Unverifiable;
            out.why = Some(format!("its base's tree could not be read: {}", e.what));
            return Ok(Some(out));
        }
    };
    let owed = declared.as_ref().and_then(|d| d.0.as_ref());
    let Some(policy) = owed.or(site.head_policy) else {
        out.compared = Compared::Lied;
        out.why =
            Some("HEAD carries no readable policy file, and no root is judged without one".into());
        return Ok(Some(out));
    };
    let own = match site.head_policy {
        Some(p) => Some(
            facts(&p.policy, root.commit, root.tree)
                .map_err(Refusal::internal)?
                .policy,
        ),
        None => None,
    };
    let mut facts = facts(&policy.policy, root.commit, root.tree).map_err(Refusal::internal)?;
    if root.policy.hash != facts.policy {
        out.compared = Compared::Lied;
        out.why = Some(format!(
            "it names policy {}, and this gate computes {} for the policy it should have judged \
             under",
            root.policy.hash, facts.policy
        ));
        return Ok(Some(out));
    }
    if site.keyed
        && owed.is_some()
        && own != Some(facts.policy)
        && facts.policy != site.ci.facts.policy
    {
        out.compared = Compared::Unverifiable;
        out.why = Some(
            "its declared base names a policy that is neither the head's own nor the branch's, \
             and the job that holds the CI key runs no other policy's checks; the pull request \
             job compares it"
                .into(),
        );
        return Ok(Some(out));
    }
    let adoption = match &root.from {
        None => judge::adoption(site.repo, &site.head)?,
        Some(from) => {
            let Some(scope) = scope(site, root, from)? else {
                out.compared = Compared::Unverifiable;
                out.why = Some(format!(
                    "its scope needs history this clone lacks: {} or its merge base with the \
                     root's base is not held",
                    from.head.hex()
                ));
                return Ok(Some(out));
            };
            facts.waivable = scope;
            declared.as_ref().map_or(Adoption::None, |d| d.1)
        }
    };
    let side = side_of(site, root, policy, &facts, &base, adoption)?;
    out.compared = compare(
        root,
        &Run {
            gate_version,
            policy: facts.policy,
            state: side.state,
            counts: side.counts.clone(),
        },
    );
    out.run = Some(side);
    Ok(Some(out))
}

/// This gate's run of the root's diff from `base`: the rules on it, the checks on the head's
/// tree, the root's levers as it applied them, its `ts` as `now`.
fn side_of(
    site: &Site,
    root: &Root,
    policy: &Loaded,
    facts: &Facts,
    base: &str,
    adoption: Adoption,
) -> Result<Side, Refusal> {
    let read = changes(site.repo, base, &site.head.commit.hex()).map_err(|e| e.to_string());
    let node = |body: Body| Node {
        tree: root.tree,
        policy: facts.policy,
        ts: root.ts,
        body,
    };
    let integrity = node(Body::Check(judge::integrity_body(
        read.as_ref().err().map(String::as_str),
        0,
    )));
    let check_id = integrity.id().map_err(Refusal::internal)?;
    let mut nodes = vec![integrity];
    let report = read
        .as_ref()
        .ok()
        .map(|c| report(&policy.policy.integrity, &policy.matchers, c, adoption));
    for f in report.iter().flat_map(|r| &r.findings) {
        nodes.push(node(Body::Finding(FindingBody {
            check: check_id,
            rule: f.rule.id().to_owned(),
            severity: f.severity,
            path: f.path.clone(),
            line: f.line,
            message: f.message.clone(),
        })));
    }
    if facts.policy == site.ci.facts.policy && root.tree == site.ci.head.tree {
        let policy_check =
            |n: &&Node| matches!(&n.body, Body::Check(c) if c.id != INTEGRITY_CHECK_ID);
        nodes.extend(site.ci.nodes.iter().filter(policy_check).cloned());
    } else {
        let ctx = Context {
            cwd: site.cwd.to_path_buf(),
            capture_cap: CAPTURE_CAP,
        };
        for spec in specs(&policy.policy) {
            let checked = run_cached(&spec, &ctx, site.cache, root.tree, facts.policy, root.ts)
                .map_err(Refusal::store)?;
            nodes.push(match checked {
                Checked::Hit(n) => n,
                Checked::Ran { node: n, .. } => n,
            });
        }
    }
    nodes.extend(root.levers.iter().map(|l| l.node.clone()));
    let verdict = gate(facts, &nodes, root.ts);
    Ok(Side {
        state: verdict.state,
        counts: counts(&nodes),
    })
}

/// What a `from` root's declared `base` says, as the merge job read it then (DESIGN.md
/// section 15): its policy when the diff from it touches `.jtac/**` and it holds one, `None`
/// meaning HEAD's, and its adoption fact. Never the verifier's base branch, which has moved on.
/// A tree there that does not read is the caller's `Unverifiable`, never a refusal: CI's own
/// verdict does not depend on it.
fn declared(site: &Site, base: &str) -> Result<(Option<Loaded>, Adoption), Refusal> {
    let at = repo::peel(site.repo, site.format, base).map_err(|e| Refusal::repository(&e))?;
    let policy = if ci::touches_jtac(site.repo, base, &site.head.commit.hex())? {
        policy::load_if_present(site.repo, &at)?
    } else {
        None
    };
    let adoption = ci::adoption_of(&Signers::at(site.repo, &at, base)?);
    Ok((policy, adoption))
}

/// The commits a `from` root's commit waiver may name: after the merge base of its base and
/// `from` up to `from`; `None` when the clone lacks that history.
fn scope(site: &Site, root: &Root, from: &FromHead) -> Result<Option<Vec<GitOid>>, Refusal> {
    let Some(base) = root.base else {
        return Ok(None);
    };
    if range::kinds(site.repo, &[from.head])?[0].as_deref() != Some("commit") {
        return Ok(None);
    }
    let merge_base = repo::probe(site.repo, &["merge-base", &base.hex(), &from.head.hex()], 1)
        .map_err(|e| Refusal::repository(&e))?;
    let Some(hex) = merge_base else {
        return Ok(None);
    };
    let stop = repo::oid(site.format, &hex).map_err(Refusal::internal)?;
    let set = range::reachable(site.repo, site.format, &from.head, &[stop])?;
    Ok(Some(set.into_iter().collect()))
}
