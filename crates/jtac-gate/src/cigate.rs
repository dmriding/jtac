//! What the CI gate reads for a commit (DESIGN.md section 9.5, decision 62), from its own
//! documents and never from the root a row cites: the commit's own root, then a CI root
//! that reaches it, then a failure record that reaches it. A document counts only when
//! trusted whole, so where no trust is read the CI gate reads nothing.

use std::collections::BTreeMap;

use jtac_core::{GitOid, State};

use crate::audit::{PolicyKind, RootFact};

/// A failure record the gatherer read, verified and found valid (section 15.2).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecordFact {
    /// The commit its verdict failed on.
    pub commit: GitOid,
    /// That commit and the first-parent line back to, and excluding, its `base`.
    pub reach: Vec<GitOid>,
    /// Trusted whole, under a signers line that names the CI namespace.
    pub trusted: bool,
}

/// The CI gate's reading of one commit. Indices are into the roots and the records.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Ci {
    /// No document of the CI gate's counts for the commit.
    Absent,
    /// A CI root, the commit's own or one that reaches it.
    Covered {
        /// That root.
        root: usize,
    },
    /// A CI root that is `declared_off`, or a CI waiver in the commit's own root.
    DeclaredOff {
        /// The root that declares it.
        root: usize,
        /// Beside a CI waiver: the CI root that reaches the commit from a later tip.
        note: Option<usize>,
    },
    /// A failure record reaches it and nothing above does.
    Failed {
        /// That record.
        record: usize,
    },
}

/// The documents that count, by the commits they speak for.
pub(crate) struct CiIndex<'a> {
    roots: &'a [RootFact],
    own: BTreeMap<GitOid, usize>,
    reach: BTreeMap<GitOid, Vec<usize>>,
    failed: BTreeMap<GitOid, Vec<usize>>,
    records: &'a [RecordFact],
}

impl<'a> CiIndex<'a> {
    pub(crate) fn new(roots: &'a [RootFact], records: &'a [RecordFact]) -> Self {
        let mut own = BTreeMap::new();
        let mut reach: BTreeMap<GitOid, Vec<usize>> = BTreeMap::new();
        for (i, r) in roots.iter().enumerate() {
            let unjudged = !r.from && r.policy == PolicyKind::NotHeld;
            if r.invalid.is_some() || !r.whole() || unjudged {
                continue;
            }
            own.insert(r.commit, i);
            for c in r.reach.iter().filter(|c| r.from && **c != r.commit) {
                reach.entry(*c).or_default().push(i);
            }
        }
        let mut failed: BTreeMap<GitOid, Vec<usize>> = BTreeMap::new();
        for (i, record) in records.iter().enumerate().filter(|(_, r)| r.trusted) {
            for c in &record.reach {
                failed.entry(*c).or_default().push(i);
            }
        }
        Self {
            roots,
            own,
            reach,
            failed,
            records,
        }
    }

    /// Whether a trusted failure record is `commit`'s own: its checks judged that tree, so
    /// the row takes no tree match (section 9.5).
    pub(crate) fn failed_here(&self, commit: &GitOid) -> bool {
        let own = |i: &usize| self.records[*i].commit == *commit;
        self.failed.get(commit).is_some_and(|v| v.iter().any(own))
    }

    /// The trusted failure record that reaches `commit`, the lowest commit's of several:
    /// `read` answers `failed` by it where nothing outranks it, and a row names it either
    /// way, as the note that a verdict failed (section 15.2).
    pub(crate) fn record(&self, commit: &GitOid) -> Option<usize> {
        let lowest = |v: &Vec<usize>| v.iter().min_by_key(|i| self.records[**i].commit).copied();
        self.failed.get(commit).and_then(lowest)
    }

    /// Decision 62's order. A waiver a person signed on the commit stays its reading; the
    /// CI root that reaches it is then the note.
    pub(crate) fn read(&self, commit: &GitOid) -> Ci {
        let covered = |i: &&usize| self.roots[**i].state == State::Covered;
        let lowest = |v: &[usize], keep: &dyn Fn(&&usize) -> bool| {
            (v.iter().filter(keep))
                .min_by_key(|i| self.roots[**i].commit)
                .copied()
        };
        let reaching = self
            .reach
            .get(commit)
            .map(Vec::as_slice)
            .unwrap_or_default();
        let reaches = lowest(reaching, &covered).or_else(|| lowest(reaching, &|_| true));
        if let Some(&root) = self.own.get(commit) {
            match (self.roots[root].from, self.roots[root].state) {
                (true, State::Covered) => return Ci::Covered { root },
                (true, _) => return Ci::DeclaredOff { root, note: None },
                (false, _) if self.roots[root].ci_waiver => {
                    return Ci::DeclaredOff {
                        root,
                        note: reaches,
                    };
                }
                (false, _) => {}
            }
        }
        if let Some(root) = reaches {
            return match self.roots[root].state {
                State::Covered => Ci::Covered { root },
                _ => Ci::DeclaredOff { root, note: None },
            };
        }
        match self.record(commit) {
            Some(record) => Ci::Failed { record },
            None => Ci::Absent,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audit::Trust;
    use crate::audit::tests::{dev, merge, oid, unknown};

    fn waived(r: RootFact) -> RootFact {
        RootFact {
            ci_waiver: true,
            waiver_trust: vec![Trust::Trusted],
            ..r
        }
    }

    fn record(commit: u8, reach: &[u8]) -> RecordFact {
        RecordFact {
            commit: oid(commit),
            reach: reach.iter().map(|c| oid(*c)).collect(),
            trusted: true,
        }
    }

    fn off(r: RootFact) -> RootFact {
        RootFact {
            state: State::DeclaredOff,
            ..r
        }
    }

    /// The commit read, the roots, the records, and the CI gate's reading of it.
    type Case = (&'static str, u8, Vec<RootFact>, Vec<RecordFact>, Ci);

    /// Each case is read again with the roots reversed: the order they are listed in
    /// decides nothing.
    fn each_way(table: Vec<Case>) {
        for (name, commit, roots, records, want) in table {
            let got = CiIndex::new(&roots, &records).read(&oid(commit));
            assert_eq!(got, want, "{name}");
            let last = roots.len().saturating_sub(1);
            let flip = |i: usize| last - i;
            let mirrored = match want {
                Ci::Covered { root } => Ci::Covered { root: flip(root) },
                Ci::DeclaredOff { root, note } => Ci::DeclaredOff {
                    root: flip(root),
                    note: note.map(flip),
                },
                other => other,
            };
            let reversed: Vec<RootFact> = roots.into_iter().rev().collect();
            let got = CiIndex::new(&reversed, &records).read(&oid(commit));
            assert_eq!(got, mirrored, "{name}, reversed");
        }
    }

    /// Section 9.5: a CI root reads its own first-parent line, and a commit's own root is
    /// read before one that reaches it (decision 62).
    #[test]
    fn the_ci_gate_reads_the_own_root_then_a_reaching_root_then_a_failure_record() {
        each_way(vec![
            (
                "a CI root reads its own commit",
                5,
                vec![merge(5, 50, &[4, 5], &[5])],
                vec![],
                Ci::Covered { root: 0 },
            ),
            (
                "and every commit of a rebase on its first-parent line",
                4,
                vec![merge(5, 50, &[3, 4, 5], &[4, 5])],
                vec![],
                Ci::Covered { root: 0 },
            ),
            (
                "never a pull request's own commit, which covers holds and reach does not",
                3,
                vec![merge(5, 50, &[3, 4, 5], &[4, 5])],
                vec![],
                Ci::Absent,
            ),
            (
                "a declared off CI root declares its reach off",
                4,
                vec![off(merge(5, 50, &[4, 5], &[4, 5]))],
                vec![],
                Ci::DeclaredOff {
                    root: 0,
                    note: None,
                },
            ),
            (
                "a covered reaching root before a declared off one, whichever id is lower",
                4,
                vec![
                    off(merge(5, 50, &[4, 5], &[4, 5])),
                    merge(6, 60, &[4, 5, 6], &[4, 5, 6]),
                ],
                vec![],
                Ci::Covered { root: 1 },
            ),
            (
                "a CI waiver in the commit's own root declares a bypass merge off",
                5,
                vec![waived(dev(5, 50, &[5]))],
                vec![],
                Ci::DeclaredOff {
                    root: 0,
                    note: None,
                },
            ),
            (
                "and stays its reading when a later tip's CI root reaches the commit",
                5,
                vec![waived(dev(5, 50, &[5])), merge(6, 60, &[5, 6], &[5, 6])],
                vec![],
                Ci::DeclaredOff {
                    root: 0,
                    note: Some(1),
                },
            ),
            (
                "a developer's own root with no CI waiver leaves the commit to the root that reaches it",
                5,
                vec![dev(5, 50, &[5]), merge(6, 60, &[5, 6], &[5, 6])],
                vec![],
                Ci::Covered { root: 1 },
            ),
            (
                "a bypass merge with a developer root and no waiver is absent to the CI gate",
                5,
                vec![dev(5, 50, &[5]), dev(4, 50, &[4])],
                vec![],
                Ci::Absent,
            ),
            (
                "a CI root over the same tree is no tree match for the CI gate",
                5,
                vec![merge(6, 50, &[6], &[6])],
                vec![],
                Ci::Absent,
            ),
        ]);
    }

    /// Each document counts only when trusted whole and valid.
    #[test]
    fn a_document_the_trust_in_force_does_not_name_whole_gives_the_ci_gate_nothing() {
        each_way(vec![
            (
                "a trusted CI waiver beside a waiver an unknown key signed declares nothing off",
                5,
                vec![RootFact {
                    waiver_trust: vec![Trust::Trusted, unknown()],
                    ..waived(dev(5, 50, &[5]))
                }],
                vec![],
                Ci::Absent,
            ),
            (
                "a CI root under an unknown key gives nothing",
                5,
                vec![RootFact {
                    trust: unknown(),
                    ..merge(5, 50, &[5], &[5])
                }],
                vec![],
                Ci::Absent,
            ),
            (
                "an invalid CI root gives nothing",
                5,
                vec![RootFact {
                    invalid: Some("its signature does not hold".into()),
                    ..merge(5, 50, &[5], &[5])
                }],
                vec![],
                Ci::Absent,
            ),
            (
                "a waiver in a root whose policy this clone lacks is not read",
                5,
                vec![RootFact {
                    policy: PolicyKind::NotHeld,
                    ..waived(dev(5, 50, &[5]))
                }],
                vec![],
                Ci::Absent,
            ),
        ]);
    }

    /// Section 15.2: a CI root and a CI waiver outrank a failure record, and nothing else.
    #[test]
    fn a_failure_record_reads_failed_where_no_ci_root_and_no_ci_waiver_counts() {
        each_way(vec![
            (
                "a failure record reads its own commit failed",
                5,
                vec![],
                vec![record(5, &[4, 5])],
                Ci::Failed { record: 0 },
            ),
            (
                "and every commit of the failed rebase",
                4,
                vec![],
                vec![record(5, &[4, 5])],
                Ci::Failed { record: 0 },
            ),
            (
                "no developer root outranks it",
                5,
                vec![dev(5, 50, &[5])],
                vec![record(5, &[5])],
                Ci::Failed { record: 0 },
            ),
            (
                "a CI waiver under an unknown key outranks no failure record",
                5,
                vec![RootFact {
                    waiver_trust: vec![unknown()],
                    ..waived(dev(5, 50, &[5]))
                }],
                vec![record(5, &[5])],
                Ci::Failed { record: 0 },
            ),
            (
                "a trusted CI waiver outranks it",
                5,
                vec![waived(dev(5, 50, &[5]))],
                vec![record(5, &[5])],
                Ci::DeclaredOff {
                    root: 0,
                    note: None,
                },
            ),
            (
                "a later attempt that passes reads covered beside the record",
                5,
                vec![merge(5, 50, &[5], &[5])],
                vec![record(5, &[5])],
                Ci::Covered { root: 0 },
            ),
            (
                "a record under an unknown key reads nothing",
                5,
                vec![],
                vec![RecordFact {
                    trusted: false,
                    ..record(5, &[5])
                }],
                Ci::Absent,
            ),
        ]);
    }

    #[test]
    fn where_no_trust_is_read_the_ci_gate_reads_nothing() {
        let unchecked = |r: RootFact| RootFact {
            trust: Trust::Unchecked,
            ..r
        };
        let roots = [
            unchecked(merge(5, 50, &[5], &[5])),
            unchecked(waived(dev(6, 60, &[6]))),
        ];
        let index = CiIndex::new(&roots, &[]);
        assert_eq!(index.read(&oid(5)), Ci::Absent);
        assert_eq!(index.read(&oid(6)), Ci::Absent);
    }
}
