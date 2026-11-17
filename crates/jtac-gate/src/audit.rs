//! Coverage state per commit from the roots a clone holds (DESIGN.md sections 9.5 and
//! 11.9). The CLI gathers the commits, the roots and what verification said about each;
//! this assigns: each commit through the first root in decision 58's order, and the CI gate
//! through its own documents (`cigate`). An invalid root is absence with a reason.

use std::collections::BTreeMap;

use jtac_core::{GitOid, Sha256Digest, Signer, State};

use crate::cigate::{Ci, CiIndex, RecordFact};

/// The `.jtac/policy.toml` of one tree, as far as coverage asks (DESIGN.md section 15.2).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PolicyKind {
    /// No file, or one this reader can read that holds no `ci` check.
    Run,
    /// It holds a `ci` check, or this reader cannot read it.
    Ci,
    /// The clone does not hold the blob or the tree that names it (decision 60).
    NotHeld,
}

/// One commit of the range, in the order the rows come out.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommitFact {
    /// The commit.
    pub commit: GitOid,
    /// Its tree.
    pub tree: GitOid,
    /// The policy in that tree.
    pub policy: PolicyKind,
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
    /// It has `from`: a CI root, whose reading is never partial.
    pub from: bool,
    /// For a root with `from`: its commit and the first-parent line back to, and excluding,
    /// its `base`, what the CI gate reads it for (section 9.5).
    pub reach: Vec<GitOid>,
    /// The policy in the tree the root names. A root without `from` is partial unless it
    /// is `Run`; one with `from` is not asked.
    pub policy: PolicyKind,
    /// A root without `from` that carries a CI waiver naming its commit.
    pub ci_waiver: bool,
    /// Who signed.
    pub signer: Option<Signer>,
    /// Whether the trust file names the signer.
    pub trust: Trust,
    /// One per `levers` entry, in order: whether the trust file names the waiver's signer.
    /// A waiver whose signature does not hold made the root invalid instead.
    pub waiver_trust: Vec<Trust>,
    /// `policy.version`.
    pub policy_version: u32,
    /// `gate.version`.
    pub gate_version: String,
    /// Why the root does not count: unreadable, wrong tree, a signature that does not
    /// hold, a `covers` entry off the chain, a parent id that disagrees, an `unread` that
    /// is not its tree's `ci` checks.
    pub invalid: Option<String>,
}

impl RootFact {
    /// Trusted whole: the trust in force names the signer and every waiver's signer.
    pub fn whole(&self) -> bool {
        self.trust == Trust::Trusted && self.waiver_trust.iter().all(|t| *t == Trust::Trusted)
    }
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

/// What a root gives a commit it bears on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reading {
    /// The gate ran every check and passed.
    Covered,
    /// A waiver turned the gate off.
    DeclaredOff,
    /// The local gate passed and left a `ci` check to CI: not covered.
    Partial,
    /// A policy the reading turns on is not in this clone: no reading, and a command that
    /// judges answers 2.
    NotHeld,
}

/// One commit's state.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    /// The commit.
    pub commit: GitOid,
    /// `None` is absent.
    pub state: Option<Reading>,
    /// Which root, when one gave the state.
    pub via: Option<Via>,
    /// Index into the roots of the one that gave the state.
    pub root: Option<usize>,
    /// Why the commit's own root did not count, when it has one that did not.
    pub invalid: Option<String>,
    /// What the CI gate reads, from its own documents.
    pub ci: Ci,
    /// For a `NotHeld` row: each tree whose policy its reading turned on, a root's or the
    /// commit's own. What a refusal names to fetch, and no other tree.
    pub lacking: Vec<GitOid>,
}

/// How a root bears on a commit, best first.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Route {
    Own,
    Covers,
    Tree,
}

/// The reading root `r` gives `c` by `route`, or `None` where it bears nothing: a partial
/// root by tree (DESIGN.md section 15.2). Coverage is granted commit by commit, so a
/// `covered` root without `from` covers through `covers` only a commit whose own policy
/// holds no `ci` check.
fn reading(r: &RootFact, route: Route, c: &CommitFact) -> Option<Reading> {
    let state = match r.state {
        State::Covered => Reading::Covered,
        State::DeclaredOff => Reading::DeclaredOff,
        State::Failed => return None,
    };
    if r.from {
        return Some(state);
    }
    match (r.policy, state, route) {
        (PolicyKind::NotHeld, ..) => Some(Reading::NotHeld),
        (PolicyKind::Ci, _, Route::Tree) => None,
        (PolicyKind::Ci, Reading::Covered, _) => Some(Reading::Partial),
        (PolicyKind::Run, Reading::Covered, Route::Covers) => Some(match c.policy {
            PolicyKind::Run => Reading::Covered,
            PolicyKind::Ci => Reading::Partial,
            PolicyKind::NotHeld => Reading::NotHeld,
        }),
        _ => Some(state),
    }
}

/// Assign (decision 58): of the valid roots that bear on a commit, a trusted one before an
/// unknown key's, a complete reading before a partial one, the commit's own, `covers`
/// before a tree match, covered before declared off, the lower commit id. A commit whose
/// own failure record is trusted takes no tree match (section 9.5). A reading that turns
/// on a policy this clone lacks makes the row `NotHeld`, whatever else bears.
pub fn rows(commits: &[CommitFact], roots: &[RootFact], records: &[RecordFact]) -> Vec<Row> {
    let mut own = BTreeMap::new();
    let mut covers: BTreeMap<GitOid, Vec<usize>> = BTreeMap::new();
    let mut trees: BTreeMap<GitOid, Vec<usize>> = BTreeMap::new();
    for (i, r) in roots.iter().enumerate() {
        own.entry(r.commit).or_insert(i);
        if r.invalid.is_some() {
            continue;
        }
        for c in &r.covers {
            covers.entry(*c).or_default().push(i);
        }
        if let Some(tree) = r.tree {
            trees.entry(tree).or_default().push(i);
        }
    }
    let ci = CiIndex::new(roots, records);
    commits
        .iter()
        .map(|c| {
            let mine = own.get(&c.commit).copied();
            let invalid = mine.and_then(|i| roots[i].invalid.clone());
            let by_tree = trees.get(&c.tree).filter(|_| !ci.failed_here(&c.commit));
            let bearing = (mine.iter().map(|i| (*i, Route::Own)))
                .chain(
                    covers
                        .get(&c.commit)
                        .into_iter()
                        .flatten()
                        .map(|i| (*i, Route::Covers)),
                )
                .chain(by_tree.into_iter().flatten().map(|i| (*i, Route::Tree)));
            let mut seen = Vec::new();
            let mut read = Vec::new();
            for (i, route) in bearing {
                if roots[i].invalid.is_some() || seen.contains(&i) {
                    continue;
                }
                seen.push(i);
                if let Some(reading) = reading(&roots[i], route, c) {
                    read.push((i, route, reading));
                }
            }
            let key = |&(i, route, reading): &(usize, Route, Reading)| {
                (
                    !roots[i].whole(),
                    reading == Reading::Partial,
                    route,
                    reading == Reading::DeclaredOff,
                    roots[i].commit,
                )
            };
            let found = match read.iter().any(|r| r.2 == Reading::NotHeld) {
                true => None,
                false => read.iter().min_by_key(|r| key(r)).copied(),
            };
            let lacking = (read.iter())
                .filter(|r| r.2 == Reading::NotHeld)
                .filter_map(|&(i, ..)| match roots[i].policy {
                    PolicyKind::NotHeld => roots[i].tree,
                    _ => Some(c.tree),
                })
                .collect();
            let via = |(i, route, _): (usize, Route, Reading)| match route {
                Route::Own => Via::Own,
                Route::Covers => Via::Covers {
                    tip: roots[i].commit,
                },
                Route::Tree => Via::Tree {
                    commit: roots[i].commit,
                },
            };
            Row {
                commit: c.commit,
                state: match (found, read.is_empty()) {
                    (Some((.., reading)), _) => Some(reading),
                    (None, true) => None,
                    (None, false) => Some(Reading::NotHeld),
                },
                via: found.map(via),
                root: found.map(|(i, ..)| i),
                invalid,
                ci: ci.read(&c.commit),
                lacking,
            }
        })
        .collect()
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub(crate) fn oid(n: u8) -> GitOid {
        format!("sha1:{:040x}", n).parse().unwrap()
    }

    fn fact(commit: u8, tree: u8, covers: &[u8], state: State, invalid: Option<&str>) -> RootFact {
        RootFact {
            commit: oid(commit),
            tree: Some(oid(tree)),
            id: None,
            state,
            covers: covers.iter().map(|c| oid(*c)).collect(),
            from: false,
            reach: Vec::new(),
            policy: PolicyKind::Run,
            ci_waiver: false,
            signer: None,
            trust: Trust::Unchecked,
            waiver_trust: Vec::new(),
            policy_version: 1,
            gate_version: "0.0.17".into(),
            invalid: invalid.map(str::to_owned),
        }
    }

    /// A trusted developer root, `covered`, under a policy with no `ci` check.
    pub(crate) fn dev(commit: u8, tree: u8, covers: &[u8]) -> RootFact {
        RootFact {
            trust: Trust::Trusted,
            ..fact(commit, tree, covers, State::Covered, None)
        }
    }

    /// A trusted CI root whose `covers` is the pull request and whose reach is `reach`.
    pub(crate) fn merge(commit: u8, tree: u8, covers: &[u8], reach: &[u8]) -> RootFact {
        RootFact {
            from: true,
            reach: reach.iter().map(|c| oid(*c)).collect(),
            ..dev(commit, tree, covers)
        }
    }

    pub(crate) fn unknown() -> Trust {
        Trust::Unknown {
            why: "no line names the key".into(),
        }
    }

    fn commit(commit: u8, tree: u8) -> CommitFact {
        CommitFact {
            commit: oid(commit),
            tree: oid(tree),
            policy: PolicyKind::Run,
        }
    }

    fn under_ci(commit: u8, tree: u8) -> CommitFact {
        CommitFact {
            policy: PolicyKind::Ci,
            ..self::commit(commit, tree)
        }
    }

    /// Each row as (state, the commit of the root that gave it).
    fn read(commits: &[CommitFact], roots: &[RootFact]) -> Vec<(Option<Reading>, Option<GitOid>)> {
        rows(commits, roots, &[])
            .into_iter()
            .map(|r| (r.state, r.root.map(|i| roots[i].commit)))
            .collect()
    }

    /// Rows: own root, a covers entry, a tree match where the covered root is read before
    /// the declared off one, absent, an invalid own root still covered by a later root's
    /// covers, a tree matched only by an invalid root (absent), the lower commit id winning
    /// a covers collision.
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
        let got = rows(&commits, &roots, &[]);
        let want = [
            (1, None, None, None, None),
            (
                2,
                Some(Reading::Covered),
                Some(Via::Covers { tip: oid(3) }),
                Some(0),
                None,
            ),
            (3, Some(Reading::Covered), Some(Via::Own), Some(0), None),
            (
                4,
                Some(Reading::Covered),
                Some(Via::Tree { commit: oid(9) }),
                Some(4),
                None,
            ),
            (6, None, None, None, None),
            (
                7,
                Some(Reading::Covered),
                Some(Via::Covers { tip: oid(8) }),
                Some(3),
                Some("its signature does not hold"),
            ),
            (8, Some(Reading::Covered), Some(Via::Own), Some(3), None),
            (10, None, None, None, None),
            (
                11,
                Some(Reading::Covered),
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
                    ci: Ci::Absent,
                    lacking: Vec::new(),
                },
                "commit {c}"
            );
        }
    }

    /// A commit, the roots that bear on it, its reading and the commit of the root cited.
    type Case = (
        &'static str,
        CommitFact,
        Vec<RootFact>,
        Option<Reading>,
        Option<u8>,
    );

    /// Each case is read again with the roots reversed: the order refs are listed in
    /// decides nothing (decision 58).
    fn each_way(table: Vec<Case>) {
        for (name, c, mut roots, state, by) in table {
            let want = vec![(state, by.map(oid))];
            assert_eq!(read(std::slice::from_ref(&c), &roots), want, "{name}");
            roots.reverse();
            assert_eq!(read(&[c], &roots), want, "{name}, reversed");
        }
    }

    fn partial(r: RootFact) -> RootFact {
        RootFact {
            policy: PolicyKind::Ci,
            ..r
        }
    }

    fn declared(r: RootFact) -> RootFact {
        RootFact {
            state: State::DeclaredOff,
            ..r
        }
    }

    const COVERED: Option<Reading> = Some(Reading::Covered);
    const OFF: Option<Reading> = Some(Reading::DeclaredOff);

    /// Decision 58, one row per key of the order.
    #[test]
    fn the_first_root_in_the_order_reads_the_commit_however_the_roots_are_listed() {
        each_way(vec![
            (
                "a pull request's commit under a ci check reads through the merge root",
                under_ci(2, 20),
                vec![partial(dev(2, 20, &[2])), merge(9, 90, &[2, 9], &[9])],
                COVERED,
                Some(9),
            ),
            (
                "an unknown key's own root yields to a trusted root that covers the commit",
                commit(2, 20),
                vec![
                    RootFact {
                        trust: unknown(),
                        ..dev(2, 20, &[2])
                    },
                    dev(3, 30, &[2, 3]),
                ],
                COVERED,
                Some(3),
            ),
            (
                "a root carrying a waiver an unknown key signed yields the same way",
                commit(2, 20),
                vec![
                    RootFact {
                        waiver_trust: vec![unknown()],
                        ..dev(2, 20, &[2])
                    },
                    dev(3, 30, &[2, 3]),
                ],
                COVERED,
                Some(3),
            ),
            (
                "a commit's own declared off root stays its reading beside a covering root",
                commit(2, 20),
                vec![declared(dev(2, 20, &[2])), dev(3, 30, &[2, 3])],
                OFF,
                Some(2),
            ),
            (
                "a partial own root beside a declared off merge root reads declared off",
                under_ci(2, 20),
                vec![
                    partial(dev(2, 20, &[2])),
                    declared(merge(9, 90, &[2, 9], &[9])),
                ],
                OFF,
                Some(9),
            ),
            (
                "covers before a tree match",
                commit(2, 20),
                vec![dev(1, 20, &[1]), dev(3, 30, &[2, 3])],
                COVERED,
                Some(3),
            ),
            (
                "a declared off covers entry before a covered tree match",
                commit(2, 20),
                vec![declared(dev(3, 30, &[2, 3])), dev(1, 20, &[1])],
                OFF,
                Some(3),
            ),
            (
                "of two covers entries the covered one is read, the lower id declared off",
                commit(2, 20),
                vec![declared(dev(3, 30, &[2, 3])), dev(4, 40, &[2, 4])],
                COVERED,
                Some(4),
            ),
            (
                "of two covered covers entries the lower commit id is cited",
                commit(2, 20),
                vec![dev(4, 40, &[2, 4]), dev(3, 30, &[2, 3])],
                COVERED,
                Some(3),
            ),
        ]);
    }

    /// Section 15.2: coverage is granted commit by commit, and a partial root is the local
    /// gate's evidence and no more.
    #[test]
    fn a_root_without_from_covers_only_under_a_policy_with_no_ci_check_that_this_clone_holds() {
        each_way(vec![
            (
                "the developer's root alone leaves a commit under a ci check partial",
                under_ci(2, 20),
                vec![partial(dev(2, 20, &[2]))],
                Some(Reading::Partial),
                Some(2),
            ),
            (
                "a trusted partial root before an unknown key's complete one",
                under_ci(2, 20),
                vec![
                    partial(dev(2, 20, &[2])),
                    RootFact {
                        trust: unknown(),
                        ..merge(9, 90, &[2, 9], &[9])
                    },
                ],
                Some(Reading::Partial),
                Some(2),
            ),
            (
                "a root on a child that drops the ci check does not cover the parent",
                under_ci(2, 20),
                vec![dev(3, 30, &[2, 3])],
                Some(Reading::Partial),
                Some(3),
            ),
            (
                "a partial root gives no tree match",
                under_ci(2, 20),
                vec![partial(dev(1, 20, &[1]))],
                None,
                None,
            ),
            (
                "a squash under a policy with no ci check still reads covered by tree",
                commit(2, 20),
                vec![dev(1, 20, &[1])],
                COVERED,
                Some(1),
            ),
            (
                "a partial declared off root declares off what it covers",
                under_ci(2, 20),
                vec![partial(declared(dev(3, 30, &[2, 3])))],
                OFF,
                Some(3),
            ),
            (
                "and gives no tree match",
                under_ci(2, 20),
                vec![partial(declared(dev(1, 20, &[1])))],
                None,
                None,
            ),
            (
                "a root whose policy this clone lacks is no reading, whatever else bears",
                commit(2, 20),
                vec![
                    RootFact {
                        policy: PolicyKind::NotHeld,
                        ..dev(3, 30, &[2, 3])
                    },
                    dev(2, 20, &[2]),
                ],
                Some(Reading::NotHeld),
                None,
            ),
            (
                "nor is a commit counted whose own policy is not held",
                CommitFact {
                    policy: PolicyKind::NotHeld,
                    ..commit(2, 20)
                },
                vec![dev(3, 30, &[2, 3])],
                Some(Reading::NotHeld),
                None,
            ),
            (
                "a merge root asks for neither policy",
                CommitFact {
                    policy: PolicyKind::NotHeld,
                    ..commit(2, 20)
                },
                vec![merge(9, 90, &[2, 9], &[9])],
                COVERED,
                Some(9),
            ),
        ]);
    }

    /// Issue 168: a refusal names the object to fetch for the rows it refuses on, so a
    /// row carries the trees it turned on, and a not-held tree that bears on no row of the
    /// range is in none.
    #[test]
    fn a_not_held_row_names_each_tree_its_reading_turned_on_and_no_other() {
        let lacks = |r: RootFact| RootFact {
            policy: PolicyKind::NotHeld,
            ..r
        };
        let roots = [
            lacks(dev(3, 30, &[2, 3])),
            dev(5, 50, &[4, 5]),
            lacks(dev(9, 90, &[9])),
        ];
        let commits = [
            commit(2, 20),
            CommitFact {
                policy: PolicyKind::NotHeld,
                ..commit(4, 40)
            },
            commit(5, 50),
        ];
        let got = rows(&commits, &roots, &[]);
        let read: Vec<_> = (got.iter().map(|r| (r.state, r.lacking.clone()))).collect();
        assert_eq!(
            read,
            [
                (Some(Reading::NotHeld), vec![oid(30)]),
                (Some(Reading::NotHeld), vec![oid(40)]),
                (Some(Reading::Covered), vec![]),
            ]
        );
    }

    /// Section 9.5, tree fallback: the failed verdict judged the tip's tree and no earlier
    /// one, so an earlier commit of the failed rebase keeps its tree match.
    #[test]
    fn a_commits_own_failure_record_withholds_a_tree_match_and_a_reaching_one_does_not() {
        let roots = [dev(1, 20, &[1]), dev(4, 30, &[4])];
        let record = RecordFact {
            commit: oid(6),
            reach: vec![oid(5), oid(6)],
            trusted: true,
        };
        let commits = [commit(5, 30), commit(6, 20)];
        let got = rows(&commits, &roots, std::slice::from_ref(&record));
        assert_eq!(
            (got[0].state, got[0].via.clone(), got[0].ci.clone()),
            (
                Some(Reading::Covered),
                Some(Via::Tree { commit: oid(4) }),
                Ci::Failed { record: 0 }
            )
        );
        assert_eq!(
            (got[1].state, got[1].ci.clone()),
            (None, Ci::Failed { record: 0 })
        );
        let unknown = RecordFact {
            trusted: false,
            ..record
        };
        let got = rows(&commits, &roots, &[unknown]);
        assert_eq!(
            (got[1].state, got[1].ci.clone()),
            (Some(Reading::Covered), Ci::Absent),
            "an unknown key's record withholds nothing"
        );
    }
}
