//! One unit of `jtac backtest` (DESIGN.md section 11.10): a first-parent commit judged against
//! its first parent, every finding as the rules reported it. A line of the `--rows` file. Pure.

use std::collections::BTreeSet;

use jtac_checks::diff::{Finding, Report};
use jtac_core::{GitOid, Severity};
use serde::Serialize;

use crate::range::Landed;

/// What the rules said about one unit.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Row {
    /// The commit.
    pub commit: GitOid,
    /// How many parents it names: 2 or more for a merge.
    pub parents: u64,
    /// Paths the change touched, a rename once.
    pub paths: u64,
    /// Lines the change added.
    pub added_lines: u64,
    /// Tiers a changed path matched.
    pub tiers: BTreeSet<String>,
    /// Every finding, by rule, then path, then line.
    pub findings: Vec<Finding>,
    /// Any finding at or above `block_at`.
    pub blocks: bool,
}

impl Row {
    /// The row for `unit`, whose change touched `paths` paths and drew `report`.
    pub fn of(unit: &Landed, paths: usize, report: Report, block_at: Severity) -> Self {
        Self {
            commit: unit.commit,
            parents: unit.parents.len() as u64,
            paths: paths as u64,
            added_lines: report.added_lines,
            tiers: report.tiers,
            blocks: report.findings.iter().any(|f| f.severity <= block_at),
            findings: report.findings,
        }
    }
}

#[cfg(test)]
mod tests {
    use jtac_checks::diff::Rule;

    use super::*;

    fn oid(n: u8) -> GitOid {
        format!("sha1:{:040x}", n).parse().unwrap()
    }

    fn finding(rule: Rule, severity: Severity, preexisting: Option<bool>) -> Finding {
        Finding {
            rule,
            severity,
            path: "src/a.rs".into(),
            line: 0,
            message: "m".into(),
            preexisting,
        }
    }

    #[test]
    fn a_row_blocks_when_a_finding_is_at_or_above_block_at() {
        let unit = Landed {
            commit: oid(1),
            parents: vec![oid(2), oid(3)],
        };
        let rows = [
            (Severity::Major, &[Severity::Minor][..], false),
            (Severity::Major, &[Severity::Minor, Severity::Major], true),
            (Severity::Major, &[Severity::Blocker], true),
            (Severity::Blocker, &[Severity::Major], false),
            (Severity::Minor, &[Severity::Minor], true),
            (Severity::Minor, &[], false),
        ];
        for (block_at, severities, blocks) in rows {
            let report = Report {
                findings: severities
                    .iter()
                    .map(|s| finding(Rule::Prot001, *s, None))
                    .collect(),
                added_lines: 7,
                tiers: BTreeSet::from(["high".to_owned()]),
            };
            let row = Row::of(&unit, 3, report, block_at);
            assert_eq!(row.blocks, blocks, "{block_at:?} {severities:?}");
            assert_eq!((row.parents, row.paths, row.added_lines), (2, 3, 7));
        }
    }

    #[test]
    fn a_rows_line_names_the_rule_by_id_and_preexisting_only_where_a_rule_said_it() {
        let report = Report {
            findings: vec![
                finding(Rule::Size001, Severity::Major, Some(true)),
                finding(Rule::Test001, Severity::Blocker, None),
            ],
            ..Report::default()
        };
        let unit = Landed {
            commit: oid(1),
            parents: vec![oid(2)],
        };
        let line = serde_json::to_string(&Row::of(&unit, 1, report, Severity::Major)).unwrap();
        assert!(
            line.contains(&format!("\"commit\":\"{}\"", oid(1))),
            "{line}"
        );
        assert!(
            line.contains("\"rule\":\"JT-SIZE-001\",\"severity\":\"major\""),
            "{line}"
        );
        assert!(line.contains("\"preexisting\":true"), "{line}");
        assert_eq!(line.matches("preexisting").count(), 1, "{line}");
        assert!(line.contains("\"blocks\":true"), "{line}");
    }
}
