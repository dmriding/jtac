//! Every root the range touches, read once and judged (DESIGN.md section 11.9): its tree
//! against its commit's, its signature against the signers, each `covers` entry on the chain
//! from its parents to its tip, each parent's root id against the one this clone holds. A
//! root the range does not touch is read for its shape only, so a long namespace costs one
//! git child, not a signature check per root. A root for a commit the clone lacks has no tree
//! to compare and no chain to walk: it counts for a tree match only. A version 2 root's
//! `base` is an ancestor of its tip and bounds the chain its `covers` is checked against,
//! entry by entry and never for completeness (DESIGN.md section 15). A root without `from`
//! is held to the policy in the tree it names (section 15.2, decision 57); one with `from`
//! gets its reach, what the CI gate reads it for (section 9.5).

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use jtac_core::{Body, Covers, GitOid, RootVersion, Sha256Digest, State};
use jtac_gate::{PolicyKind, RootFact, Trust};
use jtac_sign::{Namespace, Tool, Verified};
use jtac_store::{Store, StoredRoot};

use crate::range::{self, Listed};
use crate::refuse::Refusal;
use crate::treepolicy::{self, TreePolicy};
use crate::{output, repo, trust::Signers};

/// The touched roots as facts, with the stored form of each readable one beside it.
pub struct Ledger {
    /// In ref order.
    pub roots: Vec<RootFact>,
    /// `None` where the ref's content was not a root.
    pub stored: Vec<Option<StoredRoot>>,
    /// Skipped refs and parent roots this clone lacks.
    pub notes: Vec<String>,
    /// The policy in each tree a reading turns on (DESIGN.md section 15.2): the tree a
    /// touched root without `from` names, and the tree of each commit of the range such a
    /// root lists in `covers`, since it covers only a commit whose own policy holds no `ci`
    /// check. No other tree is asked: one git child per tree is the cost.
    pub policies: BTreeMap<GitOid, TreePolicy>,
}

impl Ledger {
    /// `treepolicy::held` over the policies of `trees`, the ones a reading turned on: a
    /// tree nothing turned on is not asked, so the fetch names no object the answer does
    /// not need.
    pub fn held<'a>(
        &self,
        repo: &Path,
        command: &str,
        trees: impl IntoIterator<Item = &'a GitOid>,
    ) -> Result<(), Refusal> {
        let policies = trees.into_iter().filter_map(|t| self.policies.get(t));
        treepolicy::held(repo, command, policies)
    }
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
    let tree_of: BTreeMap<GitOid, GitOid> = commits.iter().map(|c| (c.commit, c.tree)).collect();
    let mut ids = BTreeMap::new();
    let mut covers = Vec::new();
    for r in &reads {
        let Ok(s) = &r.stored else {
            covers.push(Vec::new());
            continue;
        };
        ids.insert(r.listed.commit, s.root.id().map_err(Refusal::internal)?);
        covers.push(match (&s.root.covers, &s.root.from) {
            (Covers::List(list), _) => list.clone(),
            (Covers::Range { base, tip }, Some(_)) => {
                match range::kinds(repo, &[*base])?[0].as_deref() {
                    Some("commit") => range::reachable(repo, format, tip, &[*base])?
                        .into_iter()
                        .collect(),
                    _ => Vec::new(),
                }
            }
            (Covers::Range { base, tip }, None) => range::between(repo, format, base, tip)?,
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
    let kinds = range::kinds(repo, &commits)?;
    let shallow = repo::shallow(repo).map_err(|e| Refusal::repository(&e))?;
    let mut asked = BTreeSet::new();
    for &i in &touched {
        if let Ok(s) = &reads[i].stored
            && s.root.from.is_none()
        {
            asked.insert(s.root.tree);
            asked.extend(covers[i].iter().filter_map(|c| tree_of.get(c)));
        }
    }
    let mut ledger = Ledger {
        roots: Vec::new(),
        stored: Vec::new(),
        notes: Vec::new(),
        policies: treepolicy::read(repo, &asked)?,
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
                    shallow,
                    based: false,
                    ids: &ids,
                    policies: &ledger.policies,
                    notes: &mut notes,
                };
                let kind = kinds[k].as_deref();
                (judged.root(s, &covers[i], trees[k], kind)?, Some(s.clone()))
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
        from: false,
        reach: Vec::new(),
        policy: PolicyKind::Run,
        ci_waiver: false,
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
    shallow: bool,
    /// Set by `base`: the root's base is a commit this clone holds.
    based: bool,
    ids: &'a BTreeMap<GitOid, Sha256Digest>,
    policies: &'a BTreeMap<GitOid, TreePolicy>,
    notes: &'a mut Vec<String>,
}

impl Judge<'_> {
    /// Why a version 2 root's base makes it invalid (DESIGN.md section 15): a non-commit, a
    /// commit that is not the tip's ancestor, or none on a tip with a parent. A base the
    /// clone lacks is invalid in a full clone that holds the tip, since it cannot be an
    /// ancestor there; in a shallow clone it is a note, and the root's covers go unjudged.
    fn base(
        &mut self,
        root: &jtac_core::Root,
        tree: Option<GitOid>,
        covers: &mut Vec<GitOid>,
    ) -> Result<Option<String>, Refusal> {
        let Some(base) = root.base else {
            let parented = root.v != RootVersion::V1
                && tree.is_some()
                && range::has_parent(self.repo, &root.commit)
                    .map_err(|e| Refusal::repository(&e))?;
            return Ok(parented.then(|| "it names no base and its tip has a parent".to_owned()));
        };
        Ok(match range::kinds(self.repo, &[base])?[0].as_deref() {
            None if tree.is_some() && !self.shallow => Some(format!(
                "its base {} is not in this clone, which holds its tip and is not shallow",
                base.hex()
            )),
            None => {
                covers.clear();
                self.notes.push(format!(
                    "{}'s root names a base this shallow clone does not hold; its covers are \
                     not judged, and it counts for its own commit and a tree match only",
                    root.commit.hex()
                ));
                None
            }
            Some("commit")
                if tree.is_some() && !range::ancestor(self.repo, &base, &root.commit)? =>
            {
                Some(format!(
                    "its base {} is not an ancestor of its tip",
                    base.hex()
                ))
            }
            Some("commit") => {
                self.based = true;
                None
            }
            Some(kind) => Some(format!("its base {} is a {kind}, not a commit", base.hex())),
        })
    }

    /// A root without `from` held to its tree's policy (`TreePolicy::refuses`), and what
    /// coverage makes of that policy.
    fn unread(&self, root: &jtac_core::Root, fact: &mut RootFact) -> Option<String> {
        let policy = self.policies.get(&root.tree)?;
        fact.policy = policy.kind();
        policy.refuses(&root.unread)
    }

    /// What the CI gate reads a root with `from` for (DESIGN.md section 9.5): its commit and
    /// the first-parent line back to its base, or its commit alone where that line does not
    /// hold the base (decision 63) or this clone does not; nothing for a commit it lacks.
    fn reach(
        &mut self,
        root: &jtac_core::Root,
        tree: Option<GitOid>,
    ) -> Result<Vec<GitOid>, Refusal> {
        Ok(match (&root.from, tree, root.base) {
            (None, ..) | (_, None, _) => Vec::new(),
            (Some(_), Some(_), Some(base)) if self.based => {
                match range::reach(self.repo, self.format, &root.commit, &base)? {
                    Some(reach) => reach,
                    None => {
                        self.notes.push(format!(
                            "{}'s root names a base that is not on its first-parent line; the \
                             CI gate reads it for its own commit alone",
                            root.commit.hex()
                        ));
                        vec![root.commit]
                    }
                }
            }
            (Some(_), Some(_), _) => vec![root.commit],
        })
    }

    /// Each waiver's signature and its signer's trust, in `levers` order; the first that
    /// does not hold is why the root is invalid. A waiver written under an agent session
    /// is a note.
    fn waivers(
        &mut self,
        root: &jtac_core::Root,
        trust: &mut Vec<Trust>,
    ) -> Result<Option<String>, Refusal> {
        for (index, lever) in root.levers.iter().enumerate() {
            let Body::Lever(body) = &lever.node.body else {
                return Ok(Some(format!("levers[{index}] is not a lever node")));
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
            trust.push(match verified {
                Verified::Trusted => Trust::Trusted,
                Verified::Unknown { why } => Trust::Unknown { why },
                Verified::Invalid { what } => {
                    return Ok(Some(format!(
                        "its waiver {index}'s signature does not hold: {what}"
                    )));
                }
            });
            if let Some(marker) = &body.agent {
                self.notes.push(format!(
                    "{}'s root applies a waiver written under an agent session ({})",
                    root.commit.hex(),
                    output::plain(marker)
                ));
            }
        }
        Ok(None)
    }

    /// Object kind, tree, base, signature in the namespace the root's shape picks, each
    /// waiver's signature, covers, parents, in that order; the first failure is the reason.
    fn root(
        mut self,
        s: &StoredRoot,
        covers: &[GitOid],
        tree: Option<GitOid>,
        kind: Option<&str>,
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
            from: root.from.is_some(),
            reach: Vec::new(),
            policy: PolicyKind::Run,
            ci_waiver: root.from.is_none()
                && (root.levers.iter())
                    .any(|l| matches!(&l.node.body, Body::Lever(b) if b.gate().is_some())),
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
            Some(_) if kind != Some("commit") => {
                return invalid(
                    fact,
                    format!(
                        "its ref names a {}, not a commit",
                        kind.unwrap_or("missing object")
                    ),
                );
            }
            Some(t) if t != root.tree => {
                return invalid(
                    fact,
                    format!("it names tree {}, not the commit's {t}", root.tree),
                );
            }
            Some(_) => {}
        }
        if let Some(why) = self.base(root, tree, &mut fact.covers)? {
            return invalid(fact, why);
        }
        if root.from.is_none()
            && let Some(why) = self.unread(root, &mut fact)
        {
            return invalid(fact, why);
        }
        fact.reach = self.reach(root, tree)?;
        let bytes = root.canonical_bytes().map_err(Refusal::internal)?;
        let sig = String::from_utf8_lossy(&s.sig);
        let verified = jtac_sign::verify(
            self.tool,
            self.signers.as_sign(),
            &root.signer,
            Namespace::of_root(root),
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
        if let Some(why) = self.waivers(root, &mut fact.waiver_trust)? {
            return invalid(fact, why);
        }
        let mut parents: Vec<GitOid> = root.parents.iter().map(|p| p.commit).collect();
        parents.extend(root.base);
        let chain = match tree {
            Some(_) => range::reachable(self.repo, self.format, &root.commit, &parents)?,
            None => BTreeSet::new(),
        };
        if let Some(off) = fact.covers.iter().find(|c| !chain.contains(c)).copied() {
            return invalid(
                fact,
                format!(
                    "it covers {}, which is not between its parents or base and its tip",
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
