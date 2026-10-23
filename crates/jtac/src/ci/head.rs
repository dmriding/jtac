//! HEAD's root compared with this gate's own run of its diff (DESIGN.md section 15,
//! Verification). The ledger judged it as a document; here the gate version and the hash of
//! the policy it should have judged under say whether a run is owed, and the run is `replay`'s
//! shape with fresh rules: the root's own base and levers, its `ts` as `now`, CI's check nodes
//! where the policy is CI's and that policy's checks run where it is not.

use std::path::Path;

use jtac_checks::diff::git::changes;
use jtac_checks::diff::{Adoption, report};
use jtac_checks::{CAPTURE_CAP, Context, run, specs};
use jtac_core::{
    Body, Compared, Counts, FindingBody, FromHead, GitOid, Node, Root, RootVersion, Run,
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
use crate::{ci, range};

/// What the comparison reads from the run around it.
pub struct Site<'a> {
    /// The repository.
    pub repo: &'a Path,
    /// `sha1` or `sha256`.
    pub format: &'a str,
    /// HEAD.
    pub head: Head,
    /// The base branch's tip.
    pub base_tip: Head,
    /// CI's adoption fact, a `from` root's too.
    pub adoption: Adoption,
    /// HEAD's policy, the one a root without `from` should have judged under.
    pub head_policy: &'a Loaded,
    /// CI's own run, whose check nodes judged the same tree.
    pub ci: &'a Judged,
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
    let owed = should_policy(site, root, &base)?;
    let policy = owed.as_ref().unwrap_or(site.head_policy);
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
            site.adoption
        }
    };
    let read = changes(site.repo, &base, &site.head.commit.hex()).map_err(|e| e.to_string());
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
    if facts.policy == site.ci.facts.policy {
        let policy_check =
            |n: &&Node| matches!(&n.body, Body::Check(c) if c.id != INTEGRITY_CHECK_ID);
        nodes.extend(site.ci.nodes.iter().filter(policy_check).cloned());
    } else {
        let ctx = Context {
            cwd: site.repo.to_path_buf(),
            capture_cap: CAPTURE_CAP,
        };
        for spec in specs(&policy.policy) {
            nodes.push(node(Body::Check(run(&spec, &ctx).body)));
        }
    }
    nodes.extend(root.levers.iter().map(|l| l.node.clone()));
    let verdict = gate(&facts, &nodes, root.ts);
    let side = Side {
        state: verdict.state,
        counts: counts(&nodes),
    };
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

/// The base branch's policy when a `from` root's diff touches `.jtac/**` and base holds one;
/// `None` means HEAD's.
fn should_policy(site: &Site, root: &Root, base: &str) -> Result<Option<Loaded>, Refusal> {
    if root.from.is_none() || !ci::touches_jtac(site.repo, base, &site.head.commit.hex())? {
        return Ok(None);
    }
    policy::load_if_present(site.repo, &site.base_tip)
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
