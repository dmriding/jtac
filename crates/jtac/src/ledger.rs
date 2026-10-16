//! Every root the range touches, read once and judged (DESIGN.md section 11.9): its tree
//! against its commit's, its signature against the signers, each `covers` entry on the chain
//! from its parents to its tip, each parent's root id against the one this clone holds. A
//! root the range does not touch is read for its shape only, so a long namespace costs one
//! git child, not a signature check per root. A root for a commit the clone lacks has no tree
//! to compare and no chain to walk: it counts for a tree match only.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use jtac_core::{Body, Covers, GitOid, Sha256Digest, State};
use jtac_gate::{RootFact, Trust};
use jtac_sign::{Namespace, Tool, Verified};
use jtac_store::{Store, StoredRoot};

use crate::range::{self, Listed};
use crate::refuse::Refusal;
use crate::trust::Signers;

/// The touched roots as facts, with the stored form of each readable one beside it.
pub struct Ledger {
    /// In ref order.
    pub roots: Vec<RootFact>,
    /// `None` where the ref's content was not a root.
    pub stored: Vec<Option<StoredRoot>>,
    /// Skipped refs and parent roots this clone lacks.
    pub notes: Vec<String>,
}

/// Read the namespace and judge the roots that bear on `commits`.
pub fn gather(
    repo: &Path,
    store: &Store,
    tool: &Tool,
    signers: &Signers,
    command: &str,
    format: &str,
    commits: &[Listed],
) -> Result<Ledger, Refusal> {
    let roots = store.roots().map_err(Refusal::store)?;
    let mut notes: Vec<String> = roots.notes().collect();
    let reads = store.read_all(&roots.listed).map_err(Refusal::store)?;
    let in_range: BTreeSet<GitOid> = commits.iter().map(|c| c.commit).collect();
    let range_trees: BTreeSet<GitOid> = commits.iter().map(|c| c.tree).collect();
    let mut ids = BTreeMap::new();
    let mut covers = Vec::new();
    for r in &reads {
        let Ok(s) = &r.stored else {
            covers.push(Vec::new());
            continue;
        };
        ids.insert(r.listed.commit, s.root.id().map_err(Refusal::internal)?);
        covers.push(match &s.root.covers {
            Covers::List(list) => list.clone(),
            Covers::Range { base, tip } => range::between(repo, format, base, tip)?,
        });
    }
    let touched: Vec<usize> = (0..reads.len())
        .filter(|&i| {
            in_range.contains(&reads[i].listed.commit)
                || covers[i].iter().any(|c| in_range.contains(c))
                || matches!(&reads[i].stored, Ok(s) if range_trees.contains(&s.root.tree))
        })
        .collect();
    let commits: Vec<GitOid> = touched.iter().map(|&i| reads[i].listed.commit).collect();
    let trees = range::trees(repo, format, &commits)?;
    let mut ledger = Ledger {
        roots: Vec::new(),
        stored: Vec::new(),
        notes: Vec::new(),
    };
    for (k, &i) in touched.iter().enumerate() {
        let (fact, stored) = match &reads[i].stored {
            Err(why) => (unreadable(reads[i].listed.commit, why), None),
            Ok(s) => {
                let judged = Judge {
                    repo,
                    tool,
                    signers,
                    command,
                    format,
                    ids: &ids,
                    notes: &mut notes,
                };
                (judged.root(s, &covers[i], trees[k])?, Some(s.clone()))
            }
        };
        ledger.roots.push(fact);
        ledger.stored.push(stored);
    }
    ledger.notes = notes;
    Ok(ledger)
}

fn unreadable(commit: GitOid, why: &str) -> RootFact {
    RootFact {
        commit,
        tree: None,
        id: None,
        state: State::Covered,
        covers: Vec::new(),
        signer: None,
        trust: Trust::Unchecked,
        waiver_trust: Vec::new(),
        policy_version: 0,
        gate_version: String::new(),
        invalid: Some(why.to_owned()),
    }
}

struct Judge<'a> {
    repo: &'a Path,
    tool: &'a Tool,
    signers: &'a Signers,
    command: &'a str,
    format: &'a str,
    ids: &'a BTreeMap<GitOid, Sha256Digest>,
    notes: &'a mut Vec<String>,
}

impl Judge<'_> {
    /// Tree, signature, each waiver's signature, covers, parents, in that order; the first
    /// failure is the reason.
    fn root(
        self,
        s: &StoredRoot,
        covers: &[GitOid],
        tree: Option<GitOid>,
    ) -> Result<RootFact, Refusal> {
        let root = &s.root;
        let mut fact = RootFact {
            commit: root.commit,
            tree: Some(root.tree),
            id: Some(root.id().map_err(Refusal::internal)?),
            state: root.state,
            covers: covers.to_vec(),
            signer: Some(root.signer.clone()),
            trust: Trust::Unchecked,
            waiver_trust: Vec::new(),
            policy_version: root.policy.version,
            gate_version: root.gate.version.clone(),
            invalid: None,
        };
        let invalid = |mut fact: RootFact, what: String| {
            fact.invalid = Some(what);
            Ok(fact)
        };
        match tree {
            None => {
                fact.covers.clear();
                self.notes.push(format!(
                    "{}'s root is for a commit this clone does not hold; it counts for a tree \
                     match only",
                    root.commit.hex()
                ));
            }
            Some(t) if t != root.tree => {
                return invalid(
                    fact,
                    format!("it names tree {}, not the commit's {t}", root.tree),
                );
            }
            Some(_) => {}
        }
        let bytes = root.canonical_bytes().map_err(Refusal::internal)?;
        let sig = String::from_utf8_lossy(&s.sig);
        let verified = jtac_sign::verify(
            self.tool,
            self.signers.as_sign(),
            &root.signer,
            Namespace::Root,
            root.ts,
            &bytes,
            &sig,
        )
        .map_err(|e| self.signers.refusal(self.command, &e))?;
        fact.trust = match verified {
            Verified::Trusted => Trust::Trusted,
            Verified::Unknown { why } => Trust::Unknown { why },
            Verified::Invalid { what } => {
                return invalid(fact, format!("its signature does not hold: {what}"));
            }
        };
        for (index, lever) in root.levers.iter().enumerate() {
            let Body::Lever(body) = &lever.node.body else {
                return invalid(fact, format!("levers[{index}] is not a lever node"));
            };
            let bytes = lever.node.canonical_bytes().map_err(Refusal::internal)?;
            let verified = jtac_sign::verify(
                self.tool,
                self.signers.as_sign(),
                &body.signer,
                Namespace::Lever,
                lever.node.ts,
                &bytes,
                &lever.sig,
            )
            .map_err(|e| self.signers.refusal(self.command, &e))?;
            fact.waiver_trust.push(match verified {
                Verified::Trusted => Trust::Trusted,
                Verified::Unknown { why } => Trust::Unknown { why },
                Verified::Invalid { what } => {
                    return invalid(
                        fact,
                        format!("its waiver {index}'s signature does not hold: {what}"),
                    );
                }
            });
            if let Some(marker) = &body.agent {
                self.notes.push(format!(
                    "{}'s root applies a waiver written under an agent session ({marker})",
                    root.commit.hex()
                ));
            }
        }
        let parents: Vec<GitOid> = root.parents.iter().map(|p| p.commit).collect();
        let chain = match tree {
            Some(_) => range::reachable(self.repo, self.format, &root.commit, &parents)?,
            None => BTreeSet::new(),
        };
        if let Some(off) = fact.covers.iter().find(|c| !chain.contains(c)).copied() {
            return invalid(
                fact,
                format!(
                    "it covers {}, which is not between its parents and its tip",
                    off.hex()
                ),
            );
        }
        for p in &root.parents {
            match self.ids.get(&p.commit) {
                Some(id) if *id != p.root => {
                    return invalid(
                        fact,
                        format!(
                            "it names {} as the root of parent {}, and this clone holds {id}",
                            p.root,
                            p.commit.hex()
                        ),
                    );
                }
                Some(_) => {}
                None => self.notes.push(format!(
                    "{}'s root links to a root for {} that this clone does not hold",
                    root.commit.hex(),
                    p.commit.hex()
                )),
            }
        }
        Ok(fact)
    }
}
