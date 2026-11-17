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
use jtac_gate::{PolicyKind, RootFact, Trust};
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
    /// The trees whose policy this gate's answer turns on (`unjudged`).
    unjudged: Vec<GitOid>,
}

impl Chain {
    /// Decision 60 for a gate that reads no row: answer 2, the fetch named, while a root
    /// its answer reads is unjudged.
    pub fn held(&self, repo: &Path, command: &str) -> Result<(), Refusal> {
        self.ledger.held(repo, command, &self.unjudged)
    }
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
    let unjudged = unjudged(&listed, &ledger.roots, &head.commit);
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
        unjudged,
    })
}

/// Whether valid root `r` judged `l`: by its own commit, its `covers`, or its tree.
fn bears(r: &RootFact, l: &range::Listed) -> bool {
    r.commit == l.commit || r.covers.contains(&l.commit) || r.tree == Some(l.tree)
}

/// The commits a valid root base trusts judged. It asks what the local gate judged, which
/// a partial root says in full, and reads no order (decision 62): section 9.5's readings
/// are not consulted here.
fn judged(listed: &[range::Listed], roots: &[RootFact]) -> BTreeSet<GitOid> {
    let trusted = |r: &&RootFact| r.invalid.is_none() && r.trust == Trust::Trusted;
    (listed.iter())
        .filter(|l| roots.iter().filter(trusted).any(|r| bears(r, l)))
        .map(|l| l.commit)
        .collect()
}

/// The tree of each root without `from` this gate's answer reads while the policy in that
/// tree is not here, so the root is not yet held to it (decisions 57 and 60): the head's
/// own root, compared whoever signed it, and each root the carry counts. A root that
/// bears on no commit of the range, or an unknown key's off the head, decides nothing and
/// asks for no fetch (`a_root_the_gate_does_not_read_asks_for_no_fetch`).
fn unjudged(listed: &[range::Listed], roots: &[RootFact], head: &GitOid) -> Vec<GitOid> {
    let read = |r: &&RootFact| {
        r.commit == *head || (r.trust == Trust::Trusted && listed.iter().any(|l| bears(r, l)))
    };
    (roots.iter())
        .filter(|r| !r.from && r.invalid.is_none() && r.policy == PolicyKind::NotHeld)
        .filter(read)
        .filter_map(|r| r.tree)
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

#[cfg(test)]
mod tests {
    use super::*;
    use jtac_core::State;

    fn oid(n: u8) -> GitOid {
        format!("sha1:{n:040x}").parse().unwrap()
    }

    /// A trusted, valid root without `from` on `commit`, its tree's policy not held.
    fn lacks(commit: u8, tree: u8, covers: &[u8]) -> RootFact {
        RootFact {
            commit: oid(commit),
            tree: Some(oid(tree)),
            id: None,
            state: State::Covered,
            covers: covers.iter().map(|c| oid(*c)).collect(),
            from: false,
            reach: Vec::new(),
            policy: PolicyKind::NotHeld,
            ci_waiver: false,
            signer: None,
            trust: Trust::Trusted,
            waiver_trust: Vec::new(),
            policy_version: 2,
            gate_version: String::new(),
            invalid: None,
        }
    }

    /// Decision 60 names the fetch for a root the answer reads and for no other: any ref
    /// under `refs/jtac/roots/` can name a tree this clone lacks, and one the gate does
    /// not read must not stop it.
    #[test]
    fn a_root_the_gate_does_not_read_asks_for_no_fetch() {
        let listed = [1, 2, 3].map(|n| range::Listed {
            commit: oid(n),
            tree: oid(n + 10),
        });
        let unknown = |r: RootFact| RootFact {
            trust: Trust::Unknown { why: "x".into() },
            ..r
        };
        let rows: [(&str, RootFact, bool); 9] = [
            ("its own commit is in the range", lacks(1, 21, &[1]), true),
            ("it covers one", lacks(9, 22, &[2, 9]), true),
            ("it judged one's tree", lacks(9, 12, &[]), true),
            ("it bears on none", lacks(9, 23, &[8, 9]), false),
            (
                "an unknown key's off the head",
                unknown(lacks(1, 24, &[1])),
                false,
            ),
            (
                "an unknown key's at the head",
                unknown(lacks(3, 25, &[3])),
                true,
            ),
            (
                "an invalid one",
                RootFact {
                    invalid: Some("x".into()),
                    ..lacks(3, 26, &[3])
                },
                false,
            ),
            (
                "one with `from`",
                RootFact {
                    from: true,
                    ..lacks(3, 27, &[3])
                },
                false,
            ),
            (
                "one whose policy is here",
                RootFact {
                    policy: PolicyKind::Ci,
                    ..lacks(3, 28, &[3])
                },
                false,
            ),
        ];
        for (name, root, asked) in rows {
            let tree = root.tree.unwrap();
            let got = unjudged(&listed, &[root], &oid(3));
            assert_eq!(got, if asked { vec![tree] } else { vec![] }, "{name}");
        }
    }
}
