//! `jtac replay` (DESIGN.md sections 7.1 and 11.9, decisions 14 and 19): the root's nodes
//! from the local store, the Merkle root, the counts and the verdict recomputed with the
//! root's `ts` as `now`, each against what the root recorded. Any mismatch invalidates it.
//! Where the nodes are not local, the root's signature is the answer and the detail is not;
//! a signature that does not hold ends it: nothing is recomputed for an invalid root. The
//! facts are the root's shape's (section 15.2): the `run` checks for a root without `from`,
//! which is held to the policy in the tree it names as every reader holds it. With
//! `--record` a failure record replays the same way and holds when its verdict fails again.

use std::collections::BTreeSet;
use std::fmt::Write;
use std::path::Path;

use jtac_core::{
    Body, Counts, GitOid, Node, Reason, Sha256Digest, Signer, State, Verdict, counts,
    from_canonical, gate, merkle_root,
};
use jtac_gate::PolicyKind;
use jtac_policy::{Checks, Policy, facts};
use jtac_sign::{Namespace, Verified};
use jtac_store::{Space, Store, StoredRoot};
use serde::Serialize;

use crate::Printed;
use crate::cli::ReplayArgs;
use crate::init::ask;
use crate::refuse::Refusal;
use crate::repo::{self, Head};
use crate::trust::Signers;
use crate::{coverage, policy, treepolicy};

/// What `jtac replay` prints.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ReplayOut {
    /// Output schema version.
    pub v: u32,
    /// The command.
    pub command: &'static str,
    /// The commit.
    pub commit: GitOid,
    /// `root` or `failure record`: which of the commit's documents was replayed.
    pub document: &'static str,
    /// Its id.
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
    let target =
        repo::peel(&repo, &format, &a.commit).map_err(|_| Refusal::rev("replay", &a.commit))?;
    let store = Store::open(&repo).map_err(Refusal::store)?;
    let (space, stored) = document(&store, &target.commit, a.record)?;
    let root = stored.root;
    let id = root.id().map_err(Refusal::internal)?;
    let tool = coverage::verifier(&repo)?;
    let signers = Signers::gather(&repo, &head, a.signers.as_deref())?;
    let bytes = root.canonical_bytes().map_err(Refusal::internal)?;
    let verified = jtac_sign::verify(
        &tool,
        signers.as_sign(),
        &root.signer,
        Namespace::of_root(&root),
        root.ts,
        &bytes,
        &String::from_utf8_lossy(&stored.sig),
    )
    .map_err(|e| signers.refusal("replay", &e))?;
    let mut mismatches = Vec::new();
    let mut notes = vec![format!("signers {}", signers.describe())];
    notes.extend(beside(&store, space, &target.commit));
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
    let (commit, noun) = (target.commit.hex(), space.noun());
    if root.tree != target.tree {
        mismatches.push(format!(
            "the {noun} names tree {}, the commit's is {}",
            root.tree, target.tree
        ));
    }
    if signature == "invalid" {
        return printed(a, unverified(&root, space, id, mismatches, notes));
    }
    if root.from.is_none() && root.tree == target.tree {
        let read = treepolicy::read(&repo, &BTreeSet::from([root.tree]))?;
        let named = &read[&root.tree];
        treepolicy::held(&repo, "replay", [named])?;
        match named.refuses(&root.unread) {
            Some(why) => mismatches.push(format!("the root is invalid: {why}")),
            None if named.kind() == PolicyKind::Ci => notes.push(
                "the policy in the tree it names leaves a check to CI: this root is the local \
                 gate's evidence, partial, and covers no commit alone"
                    .to_owned(),
            ),
            None => {}
        }
    }
    let (mut leaves, mut nodes) = local(&store, &id, noun, &commit)?;
    if !embeds(&root, &nodes) {
        let what = "`observed` is not the checks read from CI among its nodes";
        mismatches.push(format!("the {noun}'s {what}"));
    }
    for lever in &root.levers {
        let lid = lever.node.id().map_err(Refusal::internal)?;
        if !leaves.contains(&lid) {
            leaves.push(lid);
            nodes.push(lever.node.clone());
        }
    }
    let (policy_from, policy) = policy_for(&repo, &store, noun, &root.policy.hash, &target)?;
    let checks = Checks::of(root.from.is_some());
    let facts = facts(&policy, root.commit, root.tree, checks).map_err(Refusal::internal)?;
    if facts.policy != root.policy.hash {
        return Err(Refusal::policy_differs(
            noun,
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
    let state = held(space, &verdict, root.state, &mut mismatches, &mut notes);
    let out = ReplayOut {
        v: 1,
        command: "replay",
        commit: root.commit,
        document: noun,
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
            recomputed: state,
        }),
        pass: verdict.pass,
        matches: mismatches.is_empty(),
        mismatches,
        notes,
    };
    printed(a, out)
}

/// The document's nodes from this clone's store, by its manifest.
fn local(
    store: &Store,
    id: &Sha256Digest,
    noun: &str,
    commit: &str,
) -> Result<(Vec<Sha256Digest>, Vec<Node>), Refusal> {
    let absent = |what: &str| Refusal::not_local(noun, commit, what);
    let leaves = store
        .manifest(id)
        .map_err(Refusal::store)?
        .ok_or_else(|| absent("its nodes are not in this clone's store"))?;
    let mut nodes = Vec::new();
    for leaf in &leaves {
        let node = store.node(leaf).map_err(Refusal::store)?;
        let gone = || absent(&format!("node {leaf} is not in this clone's store"));
        nodes.push(node.ok_or_else(gone)?);
    }
    Ok((leaves, nodes))
}

/// The state the recomputed verdict gives a document of `space`, held to the one recorded.
/// A root holds when the verdict passes; a failure record when it fails, as the merge job
/// wrote it (`replay_and_show_read_a_failure_record_and_no_record_stops_either`).
fn held(
    space: Space,
    verdict: &Verdict,
    recorded: State,
    mismatches: &mut Vec<String>,
    notes: &mut Vec<String>,
) -> State {
    let state = match space {
        Space::Failed if !verdict.pass => State::Failed,
        _ => verdict.state,
    };
    if state != recorded {
        mismatches.push("the state differs".to_owned());
    }
    let reasons: Vec<String> = verdict.reasons.iter().map(brief).collect();
    let reasons = reasons.join("; ");
    match (space, verdict.pass) {
        (Space::Roots, false) => mismatches.push(format!("the verdict is ABORT: {reasons}")),
        (Space::Failed, false) => notes.push(format!("it failed on: {reasons}")),
        (Space::Failed, true) => mismatches.push(
            "the verdict is CLEARED HOT: a failure record holds a verdict that failed".to_owned(),
        ),
        (Space::Roots, true) => {}
    }
    state
}

impl Refusal {
    /// `replay` on a commit with no root ref.
    pub fn no_root(commit: &str, recorded: bool) -> Self {
        let (also, first) = match recorded {
            true => (
                ", and holds a failure record",
                format!("Run `jtac replay {commit} --record` to replay the failure record. "),
            ),
            false => ("", String::new()),
        };
        ask(
            format!("{commit} has no root in this clone{also}."),
            "`jtac replay` recomputes a root from its nodes; a commit without a root has \
             nothing to replay, and a commit covered by a later root is replayed through that \
             root's tip. A failure record is replayed only when asked for, so a merge whose \
             verdict failed never answers as a root does.",
            format!(
                "{first}Run `jtac audit {commit}` to see which root covers it, or `git fetch \
                 origin +refs/jtac/*:refs/jtac/*` if the root was written elsewhere."
            ),
        )
    }

    /// `replay --record` on a commit with no failure record ref.
    pub fn no_record(commit: &str) -> Self {
        ask(
            format!("{commit} has no failure record in this clone."),
            "A failure record is what the merge job writes when its verdict fails, at \
             `refs/jtac/failed/`; a commit without one has none to replay.",
            format!(
                "Run `jtac replay {commit}` for its root, `jtac audit {commit}` to see what \
                 covers it, or `git fetch origin +refs/jtac/*:refs/jtac/*` if the record was \
                 written elsewhere."
            ),
        )
    }

    /// `replay` on a root or a failure record (`noun`) whose nodes this clone never wrote.
    pub fn not_local(noun: &str, commit: &str, what: &str) -> Self {
        ask(
            format!("the {noun} for {commit} verifies, but {what}."),
            "Only roots, failure records and signatures travel; the nodes stay on the machine \
             that wrote the document, so replay there is the only full recomputation.",
            format!(
                "Run `jtac verify {commit}` here, or `jtac replay {commit}` on the machine that \
                 wrote it."
            ),
        )
    }

    /// `replay` could not rebuild the facts: the policy the root names is not here.
    pub fn policy_differs(noun: &str, commit: &str, hash: &str) -> Self {
        ask(
            format!(
                "the {noun} for {commit} names policy {hash}, which this clone cannot rebuild: \
                 the policy file at its tree resolves to another hash under this build."
            ),
            "The verdict is a function of the resolved policy; recomputing it under a policy \
             that differs would compare two different questions.",
            format!(
                "Run `jtac replay {commit}` on the machine that wrote the root, where the \
                 resolved policy is stored, or with the jtac release that wrote it."
            ),
        )
    }
}

/// Whether `root.observed` is the checks read from CI among `nodes`, each embedded once
/// (`replay_holds_a_merge_root_s_observed_to_the_checks_read_from_ci_among_its_nodes`).
fn embeds(root: &jtac_core::Root, nodes: &[Node]) -> bool {
    let read_from_ci = |n: &&Node| matches!(n.body, Body::Observed(_));
    let read: Vec<&Node> = nodes.iter().filter(read_from_ci).collect();
    read.len() == root.observed.len() && read.iter().all(|n| root.observed.contains(n))
}

/// The root, or with `--record` the failure record. A commit with no root is refused
/// though it holds a record, so a failed merge never answers 0 to a plain `replay`
/// (`replay_and_show_read_a_failure_record_and_no_record_stops_either`).
fn document(store: &Store, commit: &GitOid, record: bool) -> Result<(Space, StoredRoot), Refusal> {
    let hex = commit.hex();
    if record {
        let stored = store.record(commit).map_err(Refusal::store)?;
        let found = stored.map(|s| (Space::Failed, s));
        return found.ok_or_else(|| Refusal::no_record(&hex));
    }
    let stored = store.root(commit).map_err(Refusal::store)?;
    let recorded = matches!(store.record(commit), Ok(Some(_)));
    let found = stored.map(|s| (Space::Roots, s));
    found.ok_or_else(|| Refusal::no_root(&hex, recorded))
}

/// A note for the failure record beside a replayed root: no record stops the root's replay.
fn beside(store: &Store, space: Space, commit: &GitOid) -> Option<String> {
    let name = Space::Failed.ref_name(commit);
    match store.record(commit) {
        _ if space == Space::Failed => None,
        Ok(None) => None,
        Ok(Some(_)) => Some(format!(
            "this commit also holds a failure record at {name}; `jtac replay {} --record` \
             replays it",
            commit.hex()
        )),
        Err(why) => Some(format!(
            "{name} holds no failure record and is not read: {why}"
        )),
    }
}

/// A document whose signature does not hold: nothing recomputed, one mismatch.
fn unverified(
    root: &jtac_core::Root,
    space: Space,
    id: Sha256Digest,
    mismatches: Vec<String>,
    notes: Vec<String>,
) -> ReplayOut {
    ReplayOut {
        v: 1,
        command: "replay",
        commit: root.commit,
        document: space.noun(),
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
    noun: &str,
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
        .ok_or_else(|| Refusal::policy_differs(noun, &target.commit.hex(), &hash.to_string()))?;
    Ok(("tree", loaded.policy))
}

fn brief(r: &Reason) -> String {
    match r {
        Reason::MissingCheck { id } => format!("check `{id}` has no node"),
        Reason::DuplicateCheck { id } => format!("check `{id}` has two nodes"),
        Reason::UnknownCheck { id } => format!("check `{id}` is not in the policy"),
        Reason::UncountedCheck { id } => {
            format!("check `{id}` has a node from another source, pin or commit than its own")
        }
        Reason::CheckNotPass { id, outcome } => format!("check `{id}` is {outcome:?}"),
        Reason::Finding {
            rule, path, line, ..
        } => format!("{rule} at {path}:{line}"),
    }
}

fn render(o: &ReplayOut) -> String {
    let mut s = String::new();
    let what = match o.document {
        "root" => o.commit.hex(),
        other => format!("{}'s {other}", o.commit.hex()),
    };
    let _ = if o.state.is_none() {
        writeln!(s, "INVALID: {what} is not replayed")
    } else {
        writeln!(
            s,
            "{}: {what} replays to {} from {} nodes",
            if o.matches { "REPLAYED" } else { "MISMATCH" },
            if o.pass { "CLEARED HOT" } else { "ABORT" },
            o.nodes
        )
    };
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
            "  nothing recomputed: a {} whose signature does not hold is invalid as it stands",
            o.document
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
    fn every_replay_refusal_has_three_parts_and_names_the_next_command() {
        let rows = [
            Refusal::no_root("abc", false),
            Refusal::no_root("abc", true),
            Refusal::no_record("abc"),
            Refusal::not_local("root", "abc", "its nodes are not in this clone's store"),
            Refusal::policy_differs("failure record", "abc", "sha256:0"),
        ];
        for r in &rows {
            assert!(!r.what.is_empty() && !r.why.is_empty(), "{r:?}");
            assert!(r.how.contains("`jtac replay abc") || r.how.contains("`jtac audit abc`"));
            assert!(r.text().starts_with("jtac: "), "{r:?}");
            assert!(r.text().contains("\nnegotiation: "), "{r:?}");
            assert_eq!(r.text().lines().count(), 3, "{r:?}");
        }
        assert!(rows[1].how.starts_with("Run `jtac replay abc --record`"));
        assert!(
            rows[4]
                .what
                .starts_with("the failure record for abc names policy")
        );
    }

    #[test]
    fn a_reason_is_one_short_line_naming_its_check_or_rule() {
        let id = || "fmt".to_owned();
        let rows = [
            (Reason::MissingCheck { id: id() }, "has no node"),
            (Reason::DuplicateCheck { id: id() }, "has two nodes"),
            (Reason::UnknownCheck { id: id() }, "is not in the policy"),
            (
                Reason::UncountedCheck { id: id() },
                "has a node from another source, pin or commit than its own",
            ),
        ];
        for (reason, says) in rows {
            assert_eq!(brief(&reason), format!("check `fmt` {says}"));
        }
        let not_pass = Reason::CheckNotPass {
            id: id(),
            outcome: jtac_core::Outcome::Skipped,
        };
        assert_eq!(brief(&not_pass), "check `fmt` is Skipped");
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
