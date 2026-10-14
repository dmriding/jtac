//! `counts` derived from a root's leaves (DESIGN.md section 9.4, decision 19): one
//! implementation for the writer and for `jtac replay`, so a mismatch means the leaves changed.

use crate::node::{Action, Body, Node, Outcome, Severity};
use crate::root::Counts;

/// Totals over `nodes`. Waivers are the lever leaves with action `waive`.
pub fn counts(nodes: &[Node]) -> Counts {
    let mut c = Counts::default();
    for n in nodes {
        match &n.body {
            Body::Check(b) if b.outcome == Outcome::Pass => {
                c.checks += 1;
                c.pass += 1;
            }
            Body::Check(_) => {
                c.checks += 1;
                c.fail += 1;
            }
            Body::Finding(f) => match f.severity {
                Severity::Blocker => c.findings.blocker += 1,
                Severity::Major => c.findings.major += 1,
                Severity::Minor => c.findings.minor += 1,
            },
            Body::Lever(l) if l.action == Action::Waive => c.waivers += 1,
            Body::Lever(_) => {}
        }
    }
    c
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::node::tests::{check_node, commit_waiver};
    use crate::node::{Body, CheckBody, FindingBody};
    use crate::root::FindingCounts;

    fn check(outcome: Outcome) -> Node {
        let mut n = check_node();
        if let Body::Check(CheckBody { outcome: o, .. }) = &mut n.body {
            *o = outcome;
        }
        n
    }

    fn finding(severity: Severity) -> Node {
        let base = check_node();
        Node {
            body: Body::Finding(FindingBody {
                check: base.id().unwrap(),
                rule: "JT-X".into(),
                severity,
                path: "a".into(),
                line: 1,
                message: "m".into(),
            }),
            ..base
        }
    }

    #[test]
    fn counts_follow_outcome_severity_and_waive_action() {
        let nodes = [
            check(Outcome::Pass),
            check(Outcome::Fail),
            check(Outcome::Error),
            check(Outcome::Skipped),
            finding(Severity::Blocker),
            finding(Severity::Minor),
            finding(Severity::Minor),
            commit_waiver(),
        ];
        assert_eq!(
            counts(&nodes),
            Counts {
                checks: 4,
                pass: 1,
                fail: 3,
                findings: FindingCounts {
                    blocker: 1,
                    major: 0,
                    minor: 2
                },
                waivers: 1,
            }
        );
        assert_eq!(counts(&[]), Counts::default());
    }
}
