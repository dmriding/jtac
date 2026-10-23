//! The waivers CI's verdict is offered (DESIGN.md section 15): the levers of the chain above
//! the merge base, HEAD's own root or its nearest first-parent covered ancestor's and each
//! root's `parents` from there, each lever whose signature holds and whose signer base trusts.
//! One base does not trust is named and offered nowhere; an invalid root offers nothing.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use jtac_checks::git::git;
use jtac_core::{GitOid, SignedLever};
use jtac_gate::Trust;
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
    /// On the chain and offered nowhere, with the commit whose root carries each.
    pub untrusted: Vec<(GitOid, SignedLever)>,
    /// Roots in the range that offer nothing, and why.
    pub notes: Vec<String>,
}

/// Judge the roots over `base..head` and walk the chain for its levers.
pub fn gather(
    repo: &Path,
    store: &Store,
    tool: &Tool,
    signers: &Signers,
    format: &str,
    head: &Head,
    base: &str,
) -> Result<Chain, Refusal> {
    let span = format!("{base}..{}", head.commit.hex());
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
    let mut seen = BTreeSet::new();
    let mut trusted = Vec::new();
    let mut untrusted = Vec::new();
    while let Some(i) = queue.pop() {
        let Some(stored) = ledger.stored[i].as_ref().filter(|_| seen.insert(i)) else {
            continue;
        };
        let fact = &ledger.roots[i];
        for (lever, trust) in stored.root.levers.iter().zip(&fact.waiver_trust) {
            match trust {
                Trust::Trusted => trusted.push(lever.clone()),
                _ => untrusted.push((fact.commit, lever.clone())),
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
        untrusted,
        notes,
    })
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
