//! The verdict function (DESIGN.md section 11.2): the one place a pass or fail is decided.
//! `check` and `replay` both call `gate`; `now` is the only clock it knows.

use crate::digest::Sha256Digest;
use crate::node::{Body, LeverBody, Node, Outcome, Scope, Severity};
use crate::oid::GitOid;
use crate::root::State;
use crate::time::Ts;
use crate::waiver::{covers_finding, scope_at};

/// What the policy and the repository say; the caller gathers it, `gate` decides.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Facts {
    /// The commit under test; a commit waiver names it.
    pub commit: GitOid,
    /// The tree the checks judged.
    pub tree: GitOid,
    /// Hash of the resolved policy; evidence under another is ignored.
    pub policy: Sha256Digest,
    /// Every check the policy names, in policy order.
    pub checks: Vec<CheckSpec>,
    /// Lowest severity that fails the verdict.
    pub block_at: Severity,
}

/// One check as the policy names it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CheckSpec {
    /// The check id.
    pub id: String,
    /// Whether a non-`pass` fails the verdict.
    pub required: bool,
}

/// The decision and everything that went into it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Verdict {
    /// Whether a root may be written.
    pub pass: bool,
    /// `DeclaredOff` when a commit waiver applied.
    pub state: State,
    /// Why it would fail, in order: checks in policy order, unknown check ids in node order,
    /// then findings in node order. A commit waiver keeps them so the root stays loud.
    pub reasons: Vec<Reason>,
    /// Positions in `nodes` of the waivers that changed the outcome.
    pub applied: Vec<usize>,
}

/// One cause of failure, with the fields the CLI's three-part message needs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Reason {
    /// A required check has no node for this tree and policy.
    MissingCheck {
        /// The check id.
        id: String,
    },
    /// More than one node for one check id: ambiguous evidence.
    DuplicateCheck {
        /// The check id.
        id: String,
    },
    /// A check node for an id the policy does not name.
    UnknownCheck {
        /// The check id.
        id: String,
    },
    /// A required check's outcome is not `pass`.
    CheckNotPass {
        /// The check id.
        id: String,
        /// `fail`, `skipped`, or `error`.
        outcome: Outcome,
    },
    /// A finding at or above `block_at` with no waiver covering it.
    Finding {
        /// Rule id.
        rule: String,
        /// Severity.
        severity: Severity,
        /// Repository-relative path.
        path: String,
        /// Line.
        line: u64,
    },
}

/// Decide. Check and finding nodes on another tree or policy are ignored; waivers may be
/// older than the tree, and a repeated waiver node counts once. Blocks when
/// `severity <= block_at`: `Severity` orders Blocker first, pinned by
/// `severity_orders_blocker_first`.
pub fn gate(facts: &Facts, nodes: &[Node], now: Ts) -> Verdict {
    let here = |n: &Node| n.tree == facts.tree && n.policy == facts.policy;
    let mut reasons = Vec::new();
    let mut applied = Vec::new();
    let checks: Vec<_> = nodes
        .iter()
        .filter_map(|n| match &n.body {
            Body::Check(c) if here(n) => Some(c),
            _ => None,
        })
        .collect();
    for spec in &facts.checks {
        let id = spec.id.clone();
        let mut found = checks.iter().filter(|c| c.id == spec.id);
        match (found.next(), found.next()) {
            (None, _) if spec.required => reasons.push(Reason::MissingCheck { id }),
            (Some(_), Some(_)) => reasons.push(Reason::DuplicateCheck { id }),
            (Some(c), None) if spec.required && c.outcome != Outcome::Pass => {
                let outcome = c.outcome;
                reasons.push(Reason::CheckNotPass { id, outcome });
            }
            _ => {}
        }
    }
    for c in &checks {
        if !facts.checks.iter().any(|s| s.id == c.id) {
            reasons.push(Reason::UnknownCheck { id: c.id.clone() });
        }
    }
    let mut waivers: Vec<(usize, &LeverBody)> = Vec::new();
    for (i, n) in nodes.iter().enumerate() {
        if let Ok(w) = scope_at(n, &facts.commit, now)
            && !waivers.iter().any(|(j, _)| nodes[*j] == *n)
        {
            waivers.push((i, w));
        }
    }
    for (i, w) in &waivers {
        if matches!(w.scope, Some(Scope::Commit { .. })) {
            applied.push(*i);
        }
    }
    let findings = nodes.iter().filter_map(|n| match &n.body {
        Body::Finding(f) if here(n) && f.severity <= facts.block_at => Some(f),
        _ => None,
    });
    for f in findings {
        let covering: Vec<usize> = waivers
            .iter()
            .filter(|(_, w)| covers_finding(w, f))
            .map(|(i, _)| *i)
            .collect();
        if covering.is_empty() {
            reasons.push(Reason::Finding {
                rule: f.rule.clone(),
                severity: f.severity,
                path: f.path.clone(),
                line: f.line,
            });
        }
        applied.extend(covering);
    }
    let state = if waivers
        .iter()
        .any(|(_, w)| matches!(w.scope, Some(Scope::Commit { .. })))
    {
        State::DeclaredOff
    } else {
        State::Covered
    };
    applied.sort_unstable();
    applied.dedup();
    Verdict {
        pass: reasons.is_empty() || state == State::DeclaredOff,
        state,
        reasons,
        applied,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::node::tests::{COMMIT_ONE, EMPTY, TS, check_node, commit_waiver, rule_waiver};
    use crate::node::{CheckBody, FindingBody};

    const OTHER_TREE: &str = "sha1:0000000000000000000000000000000000000009";
    const OTHER_POLICY: &str =
        "sha256:0000000000000000000000000000000000000000000000000000000000000000";

    fn facts(checks: &[(&str, bool)], block_at: Severity) -> Facts {
        Facts {
            commit: COMMIT_ONE.parse().unwrap(),
            tree: GitOid::EMPTY_TREE_SHA1.parse().unwrap(),
            policy: EMPTY.parse().unwrap(),
            checks: checks
                .iter()
                .map(|(id, required)| CheckSpec {
                    id: (*id).into(),
                    required: *required,
                })
                .collect(),
            block_at,
        }
    }

    fn check(id: &str, outcome: Outcome) -> Node {
        let mut node = check_node();
        if let Body::Check(c) = &mut node.body {
            c.id = id.into();
            c.outcome = outcome;
        }
        node
    }

    fn finding(rule: &str, severity: Severity, path: &str) -> Node {
        Node {
            body: Body::Finding(FindingBody {
                check: EMPTY.parse().unwrap(),
                rule: rule.into(),
                severity,
                path: path.into(),
                line: 7,
                message: "example".into(),
            }),
            ..check_node()
        }
    }

    fn elsewhere(mut node: Node, tree: &str, policy: &str) -> Node {
        node.tree = tree.parse().unwrap();
        node.policy = policy.parse().unwrap();
        node
    }

    fn run(facts: &Facts, nodes: Vec<Node>) -> Verdict {
        gate(facts, &nodes, TS.parse().unwrap())
    }

    fn id(id: &str) -> String {
        id.into()
    }

    #[test]
    fn severity_orders_blocker_first() {
        assert!(Severity::Blocker < Severity::Major && Severity::Major < Severity::Minor);
    }

    #[test]
    fn required_checks_need_one_passing_node_on_this_tree_and_policy() {
        let test = facts(&[("test", true)], Severity::Major);
        let optional = facts(&[("lint", false)], Severity::Major);
        let rows = vec![
            (
                "passing node",
                &test,
                vec![check("test", Outcome::Pass)],
                vec![],
            ),
            (
                "no node",
                &test,
                vec![],
                vec![Reason::MissingCheck { id: id("test") }],
            ),
            (
                "node on another tree",
                &test,
                vec![elsewhere(check("test", Outcome::Pass), OTHER_TREE, EMPTY)],
                vec![Reason::MissingCheck { id: id("test") }],
            ),
            (
                "node under another policy",
                &test,
                vec![elsewhere(
                    check("test", Outcome::Pass),
                    GitOid::EMPTY_TREE_SHA1,
                    OTHER_POLICY,
                )],
                vec![Reason::MissingCheck { id: id("test") }],
            ),
            (
                "two nodes for one id, both passing",
                &test,
                vec![check("test", Outcome::Pass), check("test", Outcome::Pass)],
                vec![Reason::DuplicateCheck { id: id("test") }],
            ),
            (
                "an id the policy does not name",
                &test,
                vec![check("test", Outcome::Pass), check("extra", Outcome::Pass)],
                vec![Reason::UnknownCheck { id: id("extra") }],
            ),
            ("optional check missing", &optional, vec![], vec![]),
            (
                "optional check failed",
                &optional,
                vec![check("lint", Outcome::Fail)],
                vec![],
            ),
            (
                "optional check twice",
                &optional,
                vec![check("lint", Outcome::Pass), check("lint", Outcome::Pass)],
                vec![Reason::DuplicateCheck { id: id("lint") }],
            ),
        ];
        for (name, facts, nodes, want) in rows {
            let verdict = run(facts, nodes);
            assert_eq!(verdict.reasons, want, "{name}");
            assert_eq!(verdict.pass, want.is_empty(), "{name}");
            assert_eq!(verdict.state, State::Covered, "{name}");
            assert!(verdict.applied.is_empty(), "{name}");
        }
        for outcome in [Outcome::Fail, Outcome::Skipped, Outcome::Error] {
            let verdict = run(&test, vec![check("test", outcome)]);
            assert!(!verdict.pass, "{outcome:?}");
            assert_eq!(
                verdict.reasons,
                vec![Reason::CheckNotPass {
                    id: id("test"),
                    outcome
                }]
            );
        }
    }

    #[test]
    fn findings_block_at_or_above_block_at_unless_on_another_tree() {
        let rule = "JT-TEST-001";
        let rows = [
            (Severity::Major, Severity::Blocker, true),
            (Severity::Major, Severity::Major, true),
            (Severity::Major, Severity::Minor, false),
            (Severity::Minor, Severity::Minor, true),
            (Severity::Blocker, Severity::Major, false),
            (Severity::Blocker, Severity::Blocker, true),
        ];
        for (block_at, severity, blocks) in rows {
            let verdict = run(
                &facts(&[], block_at),
                vec![finding(rule, severity, "src/a.rs")],
            );
            assert_eq!(!verdict.pass, blocks, "{block_at:?} {severity:?}");
            if blocks {
                assert_eq!(
                    verdict.reasons,
                    vec![Reason::Finding {
                        rule: rule.into(),
                        severity,
                        path: "src/a.rs".into(),
                        line: 7,
                    }]
                );
            }
        }
        let away = elsewhere(
            finding(rule, Severity::Blocker, "src/a.rs"),
            OTHER_TREE,
            EMPTY,
        );
        assert!(run(&facts(&[], Severity::Minor), vec![away]).pass);
    }

    #[test]
    fn rule_waiver_covers_its_rule_under_its_path_and_counts_as_applied() {
        let f = facts(&[("test", true)], Severity::Major);
        let sup = |path: &str| finding("JT-SUP-001", Severity::Blocker, path);
        let rows = vec![
            (
                "covered finding",
                vec![sup("src/ffi/x.rs"), rule_waiver()],
                true,
                vec![1],
            ),
            (
                "finding outside the path",
                vec![sup("src/x.rs"), rule_waiver()],
                false,
                vec![],
            ),
            (
                "finding under another rule",
                vec![
                    finding("JT-TEST-001", Severity::Blocker, "src/ffi/x.rs"),
                    rule_waiver(),
                ],
                false,
                vec![],
            ),
            (
                "waiver past its expiry",
                vec![sup("src/ffi/x.rs"), expired(rule_waiver())],
                false,
                vec![],
            ),
            (
                "waiver covering only an advisory finding is not applied",
                vec![
                    finding("JT-SUP-001", Severity::Minor, "src/ffi/x.rs"),
                    rule_waiver(),
                ],
                true,
                vec![],
            ),
            (
                "one waiver, two findings, listed once",
                vec![sup("src/ffi/a.rs"), rule_waiver(), sup("src/ffi/b.rs")],
                true,
                vec![1],
            ),
            (
                "a rule waiver does not waive a failed check",
                vec![check("test", Outcome::Fail), rule_waiver()],
                false,
                vec![],
            ),
        ];
        for (name, mut nodes, pass, applied) in rows {
            if name != "a rule waiver does not waive a failed check" {
                nodes.push(check("test", Outcome::Pass));
            }
            let verdict = run(&f, nodes);
            assert_eq!(verdict.pass, pass, "{name}");
            assert_eq!(verdict.applied, applied, "{name}");
            assert_eq!(verdict.state, State::Covered, "{name}");
        }
    }

    fn expired(mut node: Node) -> Node {
        if let Body::Lever(b) = &mut node.body {
            b.expires = Some("2026-10-10T18:29:59Z".parse().unwrap());
        }
        node
    }

    #[test]
    fn commit_waiver_declares_off_and_keeps_every_reason() {
        let f = facts(&[("test", true)], Severity::Major);
        let nodes = vec![
            finding("JT-TEST-001", Severity::Blocker, "tests/a.rs"),
            commit_waiver(),
            check("test", Outcome::Error),
        ];
        let verdict = run(&f, nodes);
        assert!(verdict.pass);
        assert_eq!(verdict.state, State::DeclaredOff);
        assert_eq!(verdict.applied, vec![1]);
        assert_eq!(
            verdict.reasons,
            vec![
                Reason::CheckNotPass {
                    id: id("test"),
                    outcome: Outcome::Error
                },
                Reason::Finding {
                    rule: "JT-TEST-001".into(),
                    severity: Severity::Blocker,
                    path: "tests/a.rs".into(),
                    line: 7,
                },
            ]
        );
        let nodes = vec![
            finding("JT-SUP-001", Severity::Blocker, "src/ffi/x.rs"),
            rule_waiver(),
            rule_waiver(),
            commit_waiver(),
            check("test", Outcome::Pass),
        ];
        let verdict = run(&f, nodes);
        assert_eq!(
            (verdict.state, verdict.applied),
            (State::DeclaredOff, vec![1, 3])
        );
        let mut other = commit_waiver();
        other.ts = "2026-10-10T18:30:01Z".parse().unwrap();
        let verdict = run(&f, vec![check("test", Outcome::Error), other]);
        assert!(!verdict.pass);
        assert_eq!(verdict.state, State::Covered);
        assert!(verdict.applied.is_empty());
    }

    #[test]
    fn reasons_follow_policy_order_then_unknown_checks_then_node_order() {
        let f = facts(
            &[("fmt", true), ("lint", true), ("test", true)],
            Severity::Major,
        );
        let nodes = vec![
            finding("JT-B", Severity::Major, "b"),
            check("test", Outcome::Fail),
            check("extra", Outcome::Pass),
            finding("JT-A", Severity::Major, "a"),
            check("fmt", Outcome::Skipped),
        ];
        let kinds: Vec<String> = run(&f, nodes)
            .reasons
            .iter()
            .map(|r| match r {
                Reason::MissingCheck { id } => format!("missing {id}"),
                Reason::CheckNotPass { id, .. } => format!("notpass {id}"),
                Reason::UnknownCheck { id } => format!("unknown {id}"),
                Reason::Finding { rule, .. } => format!("finding {rule}"),
                Reason::DuplicateCheck { id } => format!("dup {id}"),
            })
            .collect();
        assert_eq!(
            kinds,
            [
                "notpass fmt",
                "missing lint",
                "notpass test",
                "unknown extra",
                "finding JT-B",
                "finding JT-A"
            ]
        );
    }

    #[test]
    fn cached_check_node_counts_like_a_fresh_one() {
        let mut node = check("test", Outcome::Pass);
        if let Body::Check(CheckBody { cached_from, .. }) = &mut node.body {
            *cached_from = Some(EMPTY.parse().unwrap());
        }
        assert!(run(&facts(&[("test", true)], Severity::Major), vec![node]).pass);
    }
}
