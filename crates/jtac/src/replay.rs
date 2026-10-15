//! `jtac replay` (DESIGN.md sections 7.1 and 11.9, decisions 14 and 19): the root's nodes
//! from the local store, the Merkle root, the counts and the verdict recomputed with the
//! root's `ts` as `now`, each against what the root recorded. Any mismatch invalidates it.
//! Where the nodes are not local, the root's signature is the answer and the detail is not;
//! a signature that does not hold ends it: nothing is recomputed for an invalid root.

use std::fmt::Write;
use std::path::Path;

use jtac_core::{
    Counts, GitOid, Reason, Sha256Digest, Signer, State, counts, from_canonical, gate, merkle_root,
};
use jtac_policy::{Policy, facts};
use jtac_sign::{Namespace, Verified};
use jtac_store::Store;
use serde::Serialize;

use crate::Printed;
use crate::cli::ReplayArgs;
use crate::refuse::Refusal;
use crate::repo::{self, Head};
use crate::trust::Signers;
use crate::{coverage, policy};

/// What `jtac replay` prints.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ReplayOut {
    /// Output schema version.
    pub v: u32,
    /// The command.
    pub command: &'static str,
    /// The commit.
    pub commit: GitOid,
    /// Its root.
    pub root: Sha256Digest,
    /// Who signed.
    pub signer: Signer,
    /// `trusted`, `unknown`, or `invalid`.
    pub signature: &'static str,
    /// `store` when the resolved policy was kept by hash, `tree` when read from the commit,
    /// `none` when nothing was recomputed because the signature does not hold.
    pub policy_from: &'static str,
    /// Leaves replayed.
    pub nodes: u64,
    /// Merkle root, recorded and recomputed; absent when nothing was recomputed.
    pub evidence: Option<Pair<Sha256Digest>>,
    /// Counts, recorded and recomputed.
    pub counts: Option<Pair<Counts>>,
    /// State, recorded and recomputed.
    pub state: Option<Pair<State>>,
    /// Whether the recomputed verdict passes; false when nothing was recomputed.
    pub pass: bool,
    /// Each disagreement.
    pub mismatches: Vec<String>,
    /// Exit 0 when true.
    pub matches: bool,
    /// What changes how to read the result.
    pub notes: Vec<String>,
}

/// A recorded value beside its recomputation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Pair<T> {
    /// In the root.
    pub recorded: T,
    /// From the nodes.
    pub recomputed: T,
}

/// Gather, recompute, render.
pub fn replay(a: &ReplayArgs, cwd: &Path) -> Result<Printed, Refusal> {
    let repo = repo::root(cwd).map_err(|e| Refusal::repository(&e))?;
    let format = repo::format(&repo).map_err(|e| Refusal::repository(&e))?;
    let head = repo::head(&repo).map_err(|e| Refusal::no_head(&e))?;
    let target = repo::peel(&repo, &format, &a.commit).map_err(|_| Refusal::rev(&a.commit))?;
    let store = Store::open(&repo).map_err(Refusal::store)?;
    let stored = store
        .root(&target.commit)
        .map_err(Refusal::store)?
        .ok_or_else(|| Refusal::no_root(&target.commit.hex()))?;
    let root = stored.root;
    let id = root.id().map_err(Refusal::internal)?;
    let tool = coverage::verifier(&repo)?;
    let signers = Signers::gather(&repo, &head, a.signers.as_deref())?;
    let bytes = root.canonical_bytes().map_err(Refusal::internal)?;
    let verified = jtac_sign::verify(
        &tool,
        signers.as_sign(),
        &root.signer,
        Namespace::Root,
        root.ts,
        &bytes,
        &String::from_utf8_lossy(&stored.sig),
    )
    .map_err(|e| signers.refusal(&e))?;
    let mut mismatches = Vec::new();
    let mut notes = vec![format!("signers {}", signers.describe())];
    let signature = match verified {
        Verified::Trusted => "trusted",
        Verified::Unknown { why } => {
            notes.push(format!("the signer is unknown: {why}"));
            "unknown"
        }
        Verified::Invalid { what } => {
            mismatches.push(format!("the signature does not hold: {what}"));
            "invalid"
        }
    };
    if root.tree != target.tree {
        mismatches.push(format!(
            "the root names tree {}, the commit's is {}",
            root.tree, target.tree
        ));
    }
    let commit = target.commit.hex();
    if signature == "invalid" {
        return printed(a, unverified(&root, id, mismatches, notes));
    }
    let mut leaves = store
        .manifest(&id)
        .map_err(Refusal::store)?
        .ok_or_else(|| Refusal::not_local(&commit, "its nodes are not in this clone's store"))?;
    let mut nodes = Vec::new();
    for leaf in &leaves {
        let node = store.node(leaf).map_err(Refusal::store)?.ok_or_else(|| {
            Refusal::not_local(
                &commit,
                &format!("node {leaf} is not in this clone's store"),
            )
        })?;
        nodes.push(node);
    }
    for lever in &root.levers {
        let lid = lever.node.id().map_err(Refusal::internal)?;
        if !leaves.contains(&lid) {
            leaves.push(lid);
            nodes.push(lever.node.clone());
        }
    }
    let (policy_from, policy) = policy_for(&repo, &store, &root.policy.hash, &target)?;
    let facts = facts(&policy, root.commit, root.tree).map_err(Refusal::internal)?;
    if facts.policy != root.policy.hash {
        return Err(Refusal::policy_differs(
            &commit,
            &root.policy.hash.to_string(),
        ));
    }
    let evidence = merkle_root(&leaves).map_err(Refusal::internal)?;
    if evidence != root.evidence {
        mismatches.push("the Merkle root differs".to_owned());
    }
    let recounted = counts(&nodes);
    if recounted != root.counts {
        mismatches.push("the counts differ".to_owned());
    }
    let verdict = gate(&facts, &nodes, root.ts);
    if verdict.state != root.state {
        mismatches.push("the state differs".to_owned());
    }
    if !verdict.pass {
        mismatches.push(format!(
            "the verdict is ABORT: {}",
            verdict
                .reasons
                .iter()
                .map(brief)
                .collect::<Vec<_>>()
                .join("; ")
        ));
    }
    let out = ReplayOut {
        v: 1,
        command: "replay",
        commit: root.commit,
        root: id,
        signer: root.signer.clone(),
        signature,
        policy_from,
        nodes: nodes.len() as u64,
        evidence: Some(Pair {
            recorded: root.evidence,
            recomputed: evidence,
        }),
        counts: Some(Pair {
            recorded: root.counts.clone(),
            recomputed: recounted,
        }),
        state: Some(Pair {
            recorded: root.state,
            recomputed: verdict.state,
        }),
        pass: verdict.pass,
        matches: mismatches.is_empty(),
        mismatches,
        notes,
    };
    printed(a, out)
}

/// A root whose signature does not hold: nothing recomputed, one mismatch.
fn unverified(
    root: &jtac_core::Root,
    id: Sha256Digest,
    mismatches: Vec<String>,
    notes: Vec<String>,
) -> ReplayOut {
    ReplayOut {
        v: 1,
        command: "replay",
        commit: root.commit,
        root: id,
        signer: root.signer.clone(),
        signature: "invalid",
        policy_from: "none",
        nodes: 0,
        evidence: None,
        counts: None,
        state: None,
        pass: false,
        matches: false,
        mismatches,
        notes,
    }
}

fn printed(a: &ReplayArgs, out: ReplayOut) -> Result<Printed, Refusal> {
    let text = if a.json {
        crate::render::json(&out)?
    } else {
        render(&out)
    };
    Ok(Printed::verdict(text, out.matches))
}

/// The resolved policy the root names: by hash from the store, else the file at the
/// commit's tree, which the caller then checks hashes the same.
fn policy_for(
    repo: &Path,
    store: &Store,
    hash: &Sha256Digest,
    target: &Head,
) -> Result<(&'static str, Policy), Refusal> {
    if let Some(bytes) = store.blob(hash).map_err(Refusal::store)? {
        let policy: Policy = from_canonical(&bytes).map_err(|e| {
            Refusal::store(format!("the stored policy {hash} does not read back: {e}"))
        })?;
        return Ok(("store", policy));
    }
    let loaded = policy::load_if_present(repo, target)?
        .ok_or_else(|| Refusal::policy_differs(&target.commit.hex(), &hash.to_string()))?;
    Ok(("tree", loaded.policy))
}

fn brief(r: &Reason) -> String {
    match r {
        Reason::MissingCheck { id } => format!("check `{id}` has no node"),
        Reason::DuplicateCheck { id } => format!("check `{id}` has two nodes"),
        Reason::UnknownCheck { id } => format!("check `{id}` is not in the policy"),
        Reason::CheckNotPass { id, outcome } => format!("check `{id}` is {outcome:?}"),
        Reason::Finding {
            rule, path, line, ..
        } => format!("{rule} at {path}:{line}"),
    }
}

fn render(o: &ReplayOut) -> String {
    let mut s = String::new();
    let _ = writeln!(
        s,
        "{}: {} replays to {} from {} nodes",
        if o.matches { "REPLAYED" } else { "MISMATCH" },
        o.commit.hex(),
        if o.pass { "CLEARED HOT" } else { "ABORT" },
        o.nodes
    );
    let _ = writeln!(s, "  root      {}", o.root);
    let _ = writeln!(
        s,
        "  signature {} ({}, {})",
        o.signature, o.signer.principal, o.signer.fingerprint
    );
    if let (Some(e), Some(c), Some(st)) = (&o.evidence, &o.counts, &o.state) {
        let _ = writeln!(s, "  policy    from the {}", o.policy_from);
        let _ = writeln!(s, "  evidence  {}", same(e.recorded == e.recomputed));
        let _ = writeln!(s, "  counts    {}", same(c.recorded == c.recomputed));
        let _ = writeln!(s, "  state     {}", same(st.recorded == st.recomputed));
    } else {
        let _ = writeln!(
            s,
            "  nothing recomputed: a root whose signature does not hold is invalid as it stands"
        );
    }
    for m in &o.mismatches {
        let _ = writeln!(s, "  mismatch  {m}");
    }
    for n in &o.notes {
        let _ = writeln!(s, "  note      {n}");
    }
    s
}

fn same(yes: bool) -> &'static str {
    if yes { "matches" } else { "DIFFERS" }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_reason_is_one_short_line_naming_its_check_or_rule() {
        assert_eq!(
            brief(&Reason::MissingCheck { id: "fmt".into() }),
            "check `fmt` has no node"
        );
        assert_eq!(
            brief(&Reason::Finding {
                rule: "JT-TEST-001".into(),
                severity: jtac_core::Severity::Major,
                path: "a.rs".into(),
                line: 3,
                message: "m".into(),
            }),
            "JT-TEST-001 at a.rs:3"
        );
    }
}
