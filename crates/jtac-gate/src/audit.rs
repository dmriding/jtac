//! Coverage state per commit from the roots a clone holds (DESIGN.md sections 9.5 and
//! 11.9). The CLI gathers the commits, the roots and what verification said about each;
//! this assigns. A commit's own root first, then a root whose `covers` names it, then a
//! valid root over the same tree, else absent. An invalid root is absence with a reason.

use std::collections::BTreeMap;

use jtac_core::{GitOid, Sha256Digest, Signer, State};

/// One commit of the range, in the order the rows come out.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommitFact {
    /// The commit.
    pub commit: GitOid,
    /// Its tree.
    pub tree: GitOid,
}

/// What the gatherer established about one root ref.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RootFact {
    /// The commit the ref is for.
    pub commit: GitOid,
    /// The tree the root names; `None` when the root could not be read.
    pub tree: Option<GitOid>,
    /// The root id; `None` when it could not be read.
    pub id: Option<Sha256Digest>,
    /// The state the root asserts.
    pub state: State,
    /// Every commit the root vouches for, the tip included, the range form expanded.
    pub covers: Vec<GitOid>,
    /// Who signed.
    pub signer: Option<Signer>,
    /// Whether the trust file names the signer.
    pub trust: Trust,
    /// `policy.version`.
    pub policy_version: u32,
    /// `gate.version`.
    pub gate_version: String,
    /// Why the root does not count: unreadable, wrong tree, a signature that does not
    /// hold, a `covers` entry off the chain, a parent id that disagrees.
    pub invalid: Option<String>,
}

/// What the trust file said about a root's signer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Trust {
    /// Named under the principal at the root's `ts`.
    Trusted,
    /// The signature holds, but no trust file names the key.
    Unknown {
        /// What ssh-keygen said, or that there is no trust file.
        why: String,
    },
    /// Not asked: the root was not verified.
    Unchecked,
}

/// Which root gave a commit its state.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Via {
    /// The commit's own root.
    Own,
    /// A root whose `covers` names the commit.
    Covers {
        /// That root's tip.
        tip: GitOid,
    },
    /// A root for another commit over the same tree.
    Tree {
        /// That root's commit.
        commit: GitOid,
    },
}

/// One commit's state.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    /// The commit.
    pub commit: GitOid,
    /// `None` is absent.
    pub state: Option<State>,
    /// Which root, when there is a state.
    pub via: Option<Via>,
    /// Index into the roots of the one that gave the state.
    pub root: Option<usize>,
    /// Why the commit's own root did not count, when it has one that did not.
    pub invalid: Option<String>,
}

/// Assign. The first valid root in `roots` order wins a `covers` or a tree match.
pub fn rows(commits: &[CommitFact], roots: &[RootFact]) -> Vec<Row> {
    let mut own = BTreeMap::new();
    let mut covers = BTreeMap::new();
    let mut trees = BTreeMap::new();
    for (i, r) in roots.iter().enumerate() {
        own.entry(r.commit).or_insert(i);
        if r.invalid.is_some() {
            continue;
        }
        for c in &r.covers {
            covers.entry(*c).or_insert(i);
        }
        if let Some(tree) = r.tree {
            trees.entry(tree).or_insert(i);
        }
    }
    commits
        .iter()
        .map(|c| {
            let invalid = own.get(&c.commit).and_then(|i| roots[*i].invalid.clone());
            let found = own
                .get(&c.commit)
                .filter(|i| roots[**i].invalid.is_none())
                .map(|i| (*i, Via::Own))
                .or_else(|| {
                    covers.get(&c.commit).map(|i| {
                        (
                            *i,
                            Via::Covers {
                                tip: roots[*i].commit,
                            },
                        )
                    })
                })
                .or_else(|| {
                    trees.get(&c.tree).map(|i| {
                        (
                            *i,
                            Via::Tree {
                                commit: roots[*i].commit,
                            },
                        )
                    })
                });
            Row {
                commit: c.commit,
                state: found.as_ref().map(|(i, _)| roots[*i].state),
                via: found.as_ref().map(|(_, via)| via.clone()),
                root: found.map(|(i, _)| i),
                invalid,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn oid(n: u8) -> GitOid {
        format!("sha1:{:040x}", n).parse().unwrap()
    }

    fn fact(commit: u8, tree: u8, covers: &[u8], state: State, invalid: Option<&str>) -> RootFact {
        RootFact {
            commit: oid(commit),
            tree: Some(oid(tree)),
            id: None,
            state,
            covers: covers.iter().map(|c| oid(*c)).collect(),
            signer: None,
            trust: Trust::Unchecked,
            policy_version: 1,
            gate_version: "0.0.17".into(),
            invalid: invalid.map(str::to_owned),
        }
    }

    fn commit(commit: u8, tree: u8) -> CommitFact {
        CommitFact {
            commit: oid(commit),
            tree: oid(tree),
        }
    }

    /// Rows: own root, a covers entry, a tree match inheriting declared_off, absent, an
    /// invalid own root still covered by a later root's covers, a tree matched only by an
    /// invalid root (absent), the first root in order winning a tree match and a covers
    /// collision.
    #[test]
    fn each_commit_takes_its_own_root_then_a_covers_entry_then_a_tree_match_else_absent() {
        let roots = [
            fact(3, 30, &[2, 3], State::Covered, None),
            fact(5, 50, &[5], State::DeclaredOff, None),
            fact(
                7,
                70,
                &[6, 7],
                State::Covered,
                Some("its signature does not hold"),
            ),
            fact(8, 80, &[7, 8], State::Covered, None),
            fact(9, 50, &[9], State::Covered, None),
            fact(12, 120, &[11, 12], State::Covered, None),
            fact(13, 130, &[11, 13], State::Covered, None),
        ];
        let commits = [
            commit(1, 10),
            commit(2, 20),
            commit(3, 30),
            commit(4, 50),
            commit(6, 60),
            commit(7, 70),
            commit(8, 80),
            commit(10, 70),
            commit(11, 110),
        ];
        let got = rows(&commits, &roots);
        let want = [
            (1, None, None, None, None),
            (
                2,
                Some(State::Covered),
                Some(Via::Covers { tip: oid(3) }),
                Some(0),
                None,
            ),
            (3, Some(State::Covered), Some(Via::Own), Some(0), None),
            (
                4,
                Some(State::DeclaredOff),
                Some(Via::Tree { commit: oid(5) }),
                Some(1),
                None,
            ),
            (6, None, None, None, None),
            (
                7,
                Some(State::Covered),
                Some(Via::Covers { tip: oid(8) }),
                Some(3),
                Some("its signature does not hold"),
            ),
            (8, Some(State::Covered), Some(Via::Own), Some(3), None),
            (10, None, None, None, None),
            (
                11,
                Some(State::Covered),
                Some(Via::Covers { tip: oid(12) }),
                Some(5),
                None,
            ),
        ];
        assert_eq!(got.len(), want.len());
        for (row, (c, state, via, root, invalid)) in got.into_iter().zip(want) {
            assert_eq!(
                row,
                Row {
                    commit: oid(c),
                    state,
                    via,
                    root,
                    invalid: invalid.map(str::to_owned),
                },
                "commit {c}"
            );
        }
    }
}
