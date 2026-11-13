//! Run every rule over a change set and order what they found (DESIGN.md section 10.3). Pure.

use std::collections::BTreeSet;

use jtac_core::Severity;
use jtac_policy::{Integrity, Matchers};
use serde::Serialize;

use super::change::Change;
use super::prot::Adoption;
use super::rule::Rule;
use super::view::View;
use super::{add, ci, cmt, prot, sec, size, sup, test};

/// One thing a rule found, without the check id `jtac check` adds.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Finding {
    /// The rule.
    pub rule: Rule,
    /// As the knobs decided it.
    pub severity: Severity,
    /// Repository-relative path.
    pub path: String,
    /// One-based line in the new version, zero when the finding has none.
    pub line: u64,
    /// The detail: what, not why.
    pub message: String,
    /// `JT-SIZE-001`, `-002` and `JT-CMT-001` only: whether the before side already broke the
    /// same cap by the same counter (DESIGN.md section 11.10). Evidence never carries it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub preexisting: Option<bool>,
}

impl Finding {
    pub(crate) fn new(
        rule: Rule,
        severity: Severity,
        path: &str,
        line: u64,
        message: impl Into<String>,
    ) -> Self {
        Self {
            rule,
            severity,
            path: path.to_owned(),
            line,
            message: message.into(),
            preexisting: None,
        }
    }

    pub(crate) fn with_preexisting(self, was: bool) -> Self {
        Self {
            preexisting: Some(was),
            ..self
        }
    }
}

/// Everything the rules said about one change set.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Report {
    /// By rule in table order, then path, then line.
    pub findings: Vec<Finding>,
    /// Lines the change added, reported whether or not `max_added_lines` is set.
    pub added_lines: u64,
    /// Tiers a changed path put the change in (DESIGN.md section 10.2).
    pub tiers: BTreeSet<String>,
}

/// Judge `changes` under `integrity` and the policy's `matchers`; `adoption` is the
/// gatherer's fact about what was in force (decisions 38 and 45).
pub fn report(
    integrity: &Integrity,
    matchers: &Matchers,
    changes: &[Change],
    adoption: Adoption,
) -> Report {
    let views: Vec<View> = changes.iter().map(View::of).collect();
    let added_lines = add::count(&views);
    let mut findings = Vec::new();
    findings.extend(prot::check(&views, matchers, adoption));
    findings.extend(test::check(&views, integrity));
    findings.extend(sup::check(&views, integrity));
    findings.extend(ci::check(&views));
    findings.extend(sec::check(&views));
    findings.extend(size::check(&views, integrity));
    findings.extend(cmt::check(&views));
    findings.extend(add::check(added_lines, integrity));
    findings.sort_by(|a, b| (a.rule, &a.path, a.line).cmp(&(b.rule, &b.path, b.line)));
    Report {
        findings,
        added_lines,
        tiers: prot::tiers(&views, matchers),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diff::change::Blob;

    #[test]
    fn findings_come_out_by_rule_then_path_then_line() {
        let policy = jtac_policy::parse("version = 1\n[integrity]\nmax_added_lines = 1\n").unwrap();
        let text = |s: &str| Some(Blob::Text(s.to_owned()));
        let changes = [
            Change {
                path: "src/b.rs".into(),
                from: None,
                old: text("#[test]\nfn t() {}\n"),
                new: text("// a\n// b\nfn x() {}\n"),
            },
            Change {
                path: ".jtac/policy.toml".into(),
                from: None,
                old: None,
                new: text("version = 1\n"),
            },
            Change {
                path: "src/a.rs".into(),
                from: None,
                old: None,
                new: text("#[allow(x)]\nfn a() {}\n"),
            },
        ];
        let got = report(
            &policy.integrity,
            &policy.matchers().unwrap(),
            &changes,
            Adoption::None,
        );
        let order: Vec<(&str, &str, u64)> = got
            .findings
            .iter()
            .map(|f| (f.rule.id(), f.path.as_str(), f.line))
            .collect();
        assert_eq!(
            order,
            [
                ("JT-PROT-001", ".jtac/policy.toml", 0),
                ("JT-TEST-001", "src/b.rs", 0),
                ("JT-SUP-001", "src/a.rs", 1),
                ("JT-CMT-001", "src/b.rs", 0),
                ("JT-ADD-001", "", 0),
            ]
        );
        assert_eq!(got.added_lines, 6);
        assert!(got.tiers.is_empty());
        let carried: Vec<bool> = got
            .findings
            .iter()
            .map(|f| f.preexisting.is_some())
            .collect();
        assert_eq!(
            carried,
            [false, false, false, true, false],
            "only size and comment findings say whether the cap was broken before"
        );
    }
}
