//! The waivers CI's verdict is offered (DESIGN.md section 15): the levers of the chain above
//! the merge base, HEAD's own root or its nearest first-parent covered ancestor's and each
//! root's `parents` from there. A lever is offered when its signature holds, base trusts its
//! signer and the root that carries it, and, for a commit waiver on an earlier commit, every
//! commit after that one is covered by a root base trusts: the signer approved that push, not
//! what was pushed around the hook after it. Anything else is named and offered nowhere; an
//! invalid root offers nothing.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use jtac_checks::git::git;
use jtac_core::{Body, GitOid, Scope, SignedLever};
use jtac_gate::{CommitFact, RootFact, Trust, rows};
use jtac_sign::Tool;
use jtac_store::Store;

use crate::ledger::{self, Ledger};
use crate::range;
use crate::refuse::Refusal;
use crate::repo::{self, Head, RepoError};
use crate::trust::Signers;

/// What the pull request's history offers.
pub struct Chain {
    /// Every root that bears on the pull request's commits, judged with base's trust.
    pub ledger: Ledger,
    /// Every commit after the merge base up to HEAD: what a commit waiver may name.
    pub commits: Vec<GitOid>,
    /// Offered to the verdict.
    pub trusted: Vec<SignedLever>,
    /// On the chain and offered nowhere.
    pub withheld: Vec<Withheld>,
    /// Roots in the range that offer nothing, and why.
    pub notes: Vec<String>,
}

/// A lever on the chain that is named and offered nowhere.
pub struct Withheld {
    /// The commit whose root carries it.
    pub commit: GitOid,
    /// The lever.
    pub lever: SignedLever,
    /// Why it is not offered.
    pub why: String,
}

/// Judge the roots over `base..head`, HEAD alone when it is parentless and its own base, and
/// walk the chain for its levers.
pub fn gather(
    repo: &Path,
    store: &Store,
    tool: &Tool,
    signers: &Signers,
    format: &str,
    head: &Head,
    base: &str,
) -> Result<Chain, Refusal> {
    let span = match base == head.commit.hex() {
        true => head.commit.hex(),
        false => format!("{base}..{}", head.commit.hex()),
    };
    let listed = range::commits(repo, format, "ci", &[span])?;
    let ledger = ledger::gather(repo, store, tool, signers, "ci", format, &listed)?;
    let commits: Vec<GitOid> = listed.iter().map(|l| l.commit).collect();
    let in_range: BTreeSet<GitOid> = commits.iter().copied().collect();
    let by_commit: BTreeMap<GitOid, usize> = ledger
        .roots
        .iter()
        .enumerate()
        .filter(|(_, r)| in_range.contains(&r.commit))
        .map(|(i, r)| (r.commit, i))
        .collect();
    let notes: Vec<String> = by_commit
        .values()
        .filter_map(|&i| {
            let r = &ledger.roots[i];
            r.invalid.as_ref().map(|why| {
                format!(
                    "{}'s root is invalid and offers no waiver: {why}",
                    r.commit.hex()
                )
            })
        })
        .collect();
    let valid = |i: &usize| ledger.roots[*i].invalid.is_none();
    let mut queue: Vec<usize> = first_parent(repo, format, head, base)?
        .iter()
        .find_map(|c| by_commit.get(c).filter(|i| valid(i)))
        .into_iter()
        .copied()
        .collect();
    let judged = judged(&listed, &ledger.roots);
    let mut seen = BTreeSet::new();
    let mut trusted = Vec::new();
    let mut withheld = Vec::new();
    while let Some(i) = queue.pop() {
        let Some(stored) = ledger.stored[i].as_ref().filter(|_| seen.insert(i)) else {
            continue;
        };
        let fact = &ledger.roots[i];
        for (lever, trust) in stored.root.levers.iter().zip(&fact.waiver_trust) {
            let why = if fact.trust != Trust::Trusted {
                Some("the root carrying it is signed by a key not in signers".to_owned())
            } else if *trust != Trust::Trusted {
                Some("its signer is not in signers".to_owned())
            } else {
                unjudged_after(repo, format, head, &in_range, &judged, lever)?
            };
            match why {
                None => trusted.push(lever.clone()),
                Some(why) => withheld.push(Withheld {
                    commit: fact.commit,
                    lever: lever.clone(),
                    why,
                }),
            }
        }
        queue.extend(
            stored
                .root
                .parents
                .iter()
                .filter_map(|p| by_commit.get(&p.commit))
                .filter(|i| valid(i)),
        );
    }
    Ok(Chain {
        ledger,
        commits,
        trusted,
        withheld,
        notes,
    })
}

/// The commits a valid root base trusts covers, by its own commit, its `covers`, or its tree.
fn judged(listed: &[range::Listed], roots: &[RootFact]) -> BTreeSet<GitOid> {
    let facts: Vec<CommitFact> = listed
        .iter()
        .map(|l| CommitFact {
            commit: l.commit,
            tree: l.tree,
        })
        .collect();
    let trusted: Vec<RootFact> = roots
        .iter()
        .filter(|r| r.invalid.is_none() && r.trust == Trust::Trusted)
        .cloned()
        .collect();
    rows(&facts, &trusted)
        .into_iter()
        .filter(|r| r.state.is_some())
        .map(|r| r.commit)
        .collect()
}

/// Why a commit waiver on an earlier commit of the pull request does not carry to HEAD: the
/// pull request's commits after the one it names that no root base trusts covers. Base-branch
/// commits merged in after it are outside the range and the base branch's own evidence. `None`
/// for a rule waiver, one naming HEAD, or one naming a commit outside the range, which the
/// verdict ignores.
fn unjudged_after(
    repo: &Path,
    format: &str,
    head: &Head,
    in_range: &BTreeSet<GitOid>,
    judged: &BTreeSet<GitOid>,
    lever: &SignedLever,
) -> Result<Option<String>, Refusal> {
    let Body::Lever(body) = &lever.node.body else {
        return Ok(None);
    };
    let Some(Scope::Commit { commit, gate: None }) = &body.scope else {
        return Ok(None);
    };
    if *commit == head.commit || !in_range.contains(commit) {
        return Ok(None);
    }
    let after = range::reachable(repo, format, &head.commit, &[*commit])?;
    let unjudged: Vec<&GitOid> = after
        .iter()
        .filter(|c| in_range.contains(c) && !judged.contains(c))
        .collect();
    Ok(unjudged.first().map(|first| {
        format!(
            "{} commits after the one it names are not covered by a root base trusts, {} among them",
            unjudged.len(),
            &first.hex()[..12]
        )
    }))
}

/// HEAD's first-parent line down to `base`, exclusive, HEAD first.
fn first_parent(
    repo: &Path,
    format: &str,
    head: &Head,
    base: &str,
) -> Result<Vec<GitOid>, Refusal> {
    let not = format!("^{base}");
    let out = git(
        repo,
        &["rev-list", "--first-parent", &head.commit.hex(), &not, "--"],
    )
    .map_err(|e| Refusal::repository(&RepoError::Git(e)))?;
    String::from_utf8_lossy(&out)
        .lines()
        .map(|hex| repo::oid(format, hex).map_err(Refusal::internal))
        .collect()
}
