//! The verdict function (DESIGN.md section 11.2): the one place a pass or fail is decided.
//! `check` and `replay` both call `gate`; `now` is the only clock it knows.

use crate::digest::Sha256Digest;
use crate::node::{Body, LeverBody, Node, Outcome, Scope, Severity};
use crate::observed::Observed;
use crate::oid::GitOid;
use crate::pin::Pin;
use crate::root::State;
use crate::time::Ts;
use crate::waiver::{Waivable, covers_finding, scope_at};

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
    /// Beside `commit`, the commits a commit waiver may name (DESIGN.md section 15): none
    /// for the local gate, the pull request's in CI.
    pub waivable: Vec<GitOid>,
}

/// One check as the policy names it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CheckSpec {
    /// The check id.
    pub id: String,
    /// Whether a non-`pass` fails the verdict.
    pub required: bool,
    /// Where CI holds the result; `None` for a check jtac runs (DESIGN.md section 15.2).
    pub pin: Option<Pin>,
}

impl CheckSpec {
    /// Whether a node from `source` (`None` for a run) is this check's on `commit`: a run
    /// never satisfies a check read from CI, nor a version 3 node a check jtac runs
    /// (`a_ci_check_counts_only_a_version_3_node_for_its_pin_and_commit`).
    pub fn counts(&self, source: Option<&Observed>, commit: &GitOid) -> bool {
        match (&self.pin, source) {
            (None, None) => true,
            (Some(pin), Some(o)) => o.is_for(pin, commit),
            _ => false,
        }
    }
}

/// The decision and everything that went into it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Verdict {
    /// Whether a root may be written.
    pub pass: bool,
    /// `DeclaredOff` when a commit waiver applied.
    pub state: State,
    /// Why it would fail, in order: checks in policy order, unknown and uncounted check
    /// nodes in node order, then findings in node order. A commit waiver keeps them so the root stays loud.
    pub reasons: Vec<Reason>,
    /// Positions in `nodes` of the levers the root carries: the waivers that changed the
    /// outcome, and a CI waiver naming the commit, which changes nothing (DESIGN.md 11.2).
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
    /// A check node its check does not count: from the other source, pin or commit.
    UncountedCheck {
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
        /// The finding's detail, so a reason names which of two at one place it is.
        message: String,
    },
}

/// Decide. Check and finding nodes on another tree or policy are ignored; waivers may be
/// older than the tree, and a repeated waiver node counts once. A check counts a node from
/// its own source alone (`CheckSpec::counts`), and a node on this tree and policy that its
/// check does not count is a reason, never ignored: one policy hash names one source, so
/// such a node is no leftover. Blocks when `severity <= block_at`: `Severity` orders Blocker
/// first, pinned by `severity_orders_blocker_first`.
pub fn gate(facts: &Facts, nodes: &[Node], now: Ts) -> Verdict {
    let here = |n: &Node| n.tree == facts.tree && n.policy == facts.policy;
    let mut reasons = Vec::new();
    let mut applied = Vec::new();
    let checks: Vec<(&str, Outcome, Option<&Observed>)> = nodes
        .iter()
        .filter_map(|n| match &n.body {
            Body::Check(c) if here(n) => Some((c.id.as_str(), c.outcome, None)),
            Body::Observed(c) if here(n) => Some((c.id.as_str(), c.outcome, Some(&c.observed))),
            _ => None,
        })
        .collect();
    for spec in &facts.checks {
        let id = spec.id.clone();
        let mut found = checks
            .iter()
            .filter(|(seen, _, source)| *seen == spec.id && spec.counts(*source, &facts.commit));
        match (found.next(), found.next()) {
            (None, _) if spec.required => reasons.push(Reason::MissingCheck { id }),
            (Some(_), Some(_)) => reasons.push(Reason::DuplicateCheck { id }),
            (Some((_, outcome, _)), None) if spec.required && *outcome != Outcome::Pass => {
                let outcome = *outcome;
                reasons.push(Reason::CheckNotPass { id, outcome });
            }
            _ => {}
        }
    }
    for (seen, _, source) in &checks {
        let id = (*seen).into();
        match facts.checks.iter().find(|s| s.id == *seen) {
            None => reasons.push(Reason::UnknownCheck { id }),
            Some(spec) if !spec.counts(*source, &facts.commit) => {
                reasons.push(Reason::UncountedCheck { id });
            }
            Some(_) => {}
        }
    }
    let mut waivers: Vec<(usize, &LeverBody)> = Vec::new();
    for (i, n) in nodes.iter().enumerate() {
        if let Ok(w) = scope_at(n, &facts.commit, Waivable::Commits(&facts.waivable), now)
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
                message: f.message.clone(),
            });
        }
        applied.extend(covering);
    }
    let state = if waivers
        .iter()
        .any(|(_, w)| matches!(w.scope, Some(Scope::Commit { gate: None, .. })))
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
    use crate::node::tests::{
        COMMIT_ONE, EMPTY, TS, check_node, ci_waiver, commit_waiver, observed_node, rule_waiver,
    };
    use crate::node::{CheckBody, FindingBody};
    use crate::observed::ObservedCheck;

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
                    pin: None,
                })
                .collect(),
            block_at,
            waivable: Vec::new(),
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
                        message: "example".into(),
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
                    message: "example".into(),
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

    /// The CI waiver changes no verdict (DESIGN.md 11.2): a failing run still fails and a
    /// passing one stays `covered`; either way the lever is carried in `applied`.
    #[test]
    fn a_ci_waiver_is_carried_and_never_applied() {
        let f = facts(&[("test", true)], Severity::Major);
        let verdict = run(&f, vec![check("test", Outcome::Error), ci_waiver()]);
        assert_eq!((verdict.pass, verdict.state), (false, State::Covered));
        assert_eq!(verdict.applied, vec![1]);
        let verdict = run(&f, vec![check("test", Outcome::Pass), ci_waiver()]);
        assert_eq!((verdict.pass, verdict.state), (true, State::Covered));
        assert_eq!(verdict.applied, vec![1]);
        let verdict = run(
            &f,
            vec![check("test", Outcome::Pass), ci_waiver(), commit_waiver()],
        );
        assert_eq!(verdict.state, State::DeclaredOff);
        assert_eq!(verdict.applied, vec![1, 2]);
    }

    #[test]
    fn a_commit_waiver_on_a_waivable_commit_declares_off_and_on_another_does_not() {
        let elsewhere: GitOid = OTHER_TREE.parse().unwrap();
        let mut other = commit_waiver();
        if let Body::Lever(b) = &mut other.body {
            b.scope = Some(Scope::Commit {
                commit: elsewhere,
                gate: None,
            });
        }
        let mut f = facts(&[("test", true)], Severity::Major);
        let nodes = vec![check("test", Outcome::Error), other];
        let verdict = run(&f, nodes.clone());
        assert_eq!((verdict.pass, verdict.state), (false, State::Covered));
        f.waivable = vec![elsewhere];
        let verdict = run(&f, nodes);
        assert_eq!((verdict.pass, verdict.state), (true, State::DeclaredOff));
        assert_eq!(verdict.applied, vec![1]);
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
                Reason::UncountedCheck { id } => format!("uncounted {id}"),
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

    /// DESIGN.md sections 11.2 and 15.2: the pin, the commit and the node's version each
    /// decide, and a check node its check does not count fails the verdict where it stands.
    #[test]
    fn a_ci_check_counts_only_a_version_3_node_for_its_pin_and_commit() {
        let mut f = facts(&[("fmt", true), ("test", true)], Severity::Major);
        f.checks[1].pin = Some(Pin {
            workflow: ".github/workflows/ci.yml".into(),
            job: Some("test".into()),
        });
        let seen = |edit: &dyn Fn(&mut ObservedCheck)| {
            let mut node = observed_node();
            if let Body::Observed(c) = &mut node.body {
                edit(c);
            }
            node
        };
        let fmt = || check("fmt", Outcome::Pass);
        let missing = || Reason::MissingCheck { id: id("test") };
        let uncounted = |of: &str| Reason::UncountedCheck { id: id(of) };
        let rows = vec![
            (
                "the pinned job passed",
                vec![fmt(), observed_node()],
                vec![],
            ),
            (
                "the pinned job failed",
                vec![fmt(), seen(&|c| c.outcome = Outcome::Fail)],
                vec![Reason::CheckNotPass {
                    id: id("test"),
                    outcome: Outcome::Fail,
                }],
            ),
            (
                "a run's node for the ci check",
                vec![fmt(), check("test", Outcome::Pass)],
                vec![missing(), uncounted("test")],
            ),
            (
                "a failing run's node beside the observed pass",
                vec![fmt(), observed_node(), check("test", Outcome::Fail)],
                vec![uncounted("test")],
            ),
            (
                "a version 3 node for the run check",
                vec![seen(&|c| c.id = "fmt".into()), observed_node()],
                vec![Reason::MissingCheck { id: id("fmt") }, uncounted("fmt")],
            ),
            (
                "another definition file",
                vec![
                    fmt(),
                    seen(&|c| c.observed.workflow = ".github/workflows/x.yml".into()),
                ],
                vec![missing(), uncounted("test")],
            ),
            (
                "another job",
                vec![fmt(), seen(&|c| c.observed.job = Some("lint".into()))],
                vec![missing(), uncounted("test")],
            ),
            (
                "the run's word where policy names a job",
                vec![fmt(), seen(&|c| c.observed.job = None)],
                vec![missing(), uncounted("test")],
            ),
            (
                "another commit",
                vec![
                    fmt(),
                    seen(&|c| c.observed.commit = OTHER_TREE.parse().unwrap()),
                ],
                vec![missing(), uncounted("test")],
            ),
            (
                "two results for the pin",
                vec![fmt(), observed_node(), seen(&|c| c.observed.run = 9)],
                vec![Reason::DuplicateCheck { id: id("test") }],
            ),
            (
                "an id the policy does not name",
                vec![fmt(), observed_node(), seen(&|c| c.id = "extra".into())],
                vec![Reason::UnknownCheck { id: id("extra") }],
            ),
            (
                "a version 3 node on another tree is not here at all",
                vec![fmt(), elsewhere(observed_node(), OTHER_TREE, EMPTY)],
                vec![missing()],
            ),
        ];
        for (name, nodes, want) in rows {
            let verdict = run(&f, nodes);
            assert_eq!(verdict.reasons, want, "{name}");
            assert_eq!(verdict.pass, want.is_empty(), "{name}");
        }
        let mut jobless = f.clone();
        jobless.checks[1].pin = Some(Pin {
            workflow: ".github/workflows/ci.yml".into(),
            job: None,
        });
        assert!(run(&jobless, vec![fmt(), seen(&|c| c.observed.job = None)]).pass);
        let verdict = run(&jobless, vec![fmt(), observed_node()]);
        assert_eq!(verdict.reasons, vec![missing(), uncounted("test")]);
        f.checks[1].required = false;
        assert!(run(&f, vec![fmt()]).pass);
        let verdict = run(&f, vec![fmt(), check("test", Outcome::Pass)]);
        assert_eq!(verdict.reasons, vec![uncounted("test")]);
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
