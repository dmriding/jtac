//! The `jtac backtest` summary (DESIGN.md section 11.10, decision 51): counts, rule ids and
//! the policy's hash only, so it can leave a private repository; shares are per mille, rounded down, since no float
//! is printed (decision 17). Pure: rows in, summary out.

use std::collections::{BTreeMap, BTreeSet};

use jtac_checks::diff::{Finding, Lang, Rule};
use jtac_core::{Severity, Sha256Digest};
use serde::Serialize;

use super::row::Row;

/// Where the run's one policy was read.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum PolicyFrom {
    /// The `--policy` file.
    File,
    /// The tree of the range's newest commit: with more than one tip, the first git walks.
    NewestCommit,
}

/// The policy a run judged under: the input that sets every count.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Under {
    /// The lowest severity that blocks.
    pub block_at: Severity,
    /// The policy's hash, as a root records it.
    pub policy: Sha256Digest,
    /// Where it was read.
    pub from: PolicyFrom,
}

/// The statement every summary carries.
pub const DOES_NOT_SAY: [&str; 2] = [
    "A finding is not a defect and a unit with no finding is not a clean one: precision needs \
     each sampled finding judged by someone who knows the change, and recall needs seeded \
     changes, since history holds only what merged.",
    "A merged diff is not a first push either, so these rates bound what CI would have said, \
     not what the local gate would.",
];

/// What `jtac backtest` prints.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Summary {
    /// Output schema version.
    pub v: u32,
    /// The command.
    pub command: &'static str,
    /// The hash of the one policy every unit was judged under.
    pub policy: Sha256Digest,
    /// Where that policy was read.
    pub policy_from: PolicyFrom,
    /// The lowest severity that blocks, from the policy.
    pub block_at: Severity,
    /// First-parent commits judged against their first parent.
    pub units: u64,
    /// Parentless commits: nothing to judge.
    pub skipped: u64,
    /// Units with a finding at or above `block_at`.
    pub blocking: u64,
    /// `blocking` per thousand units, rounded down.
    pub blocking_per_mille: u64,
    /// Units with no finding at all.
    pub silent: u64,
    /// `silent` per thousand units, rounded down.
    pub silent_per_mille: u64,
    /// Findings by rule, then severity.
    pub findings: Vec<RuleCount>,
    /// Units whose every blocking finding is of one rule, by that rule.
    pub blocked_alone: Vec<Alone>,
    /// The same two breakdowns over the findings on each language's paths.
    pub languages: Vec<ByLanguage>,
    /// Lines written to the `--rows` file, which holds paths and messages; absent without it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rows_written: Option<u64>,
    /// What these counts do not say.
    pub does_not_say: [&'static str; 2],
}

/// Findings of one rule at one severity.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct RuleCount {
    /// The rule.
    pub rule: Rule,
    /// The severity.
    pub severity: Severity,
    /// How many.
    pub findings: u64,
    /// Of those, the ones whose cap the before side already broke; absent for a rule that
    /// does not say.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub preexisting: Option<u64>,
}

/// Units blocked by one rule and no other.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Alone {
    /// The rule.
    pub rule: Rule,
    /// How many units.
    pub units: u64,
}

/// The breakdowns over the findings on one language's paths. A unit blocked by one rule
/// counts under each language its blocking findings touch.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ByLanguage {
    /// `go`, `js`, `python`, `rust`, or `other` for a path no rule table reads.
    pub language: &'static str,
    /// Findings by rule, then severity.
    pub findings: Vec<RuleCount>,
    /// Units blocked by one rule alone.
    pub blocked_alone: Vec<Alone>,
}

#[derive(Default)]
struct Tally {
    findings: BTreeMap<(Rule, Severity), (u64, Option<u64>)>,
    alone: BTreeMap<Rule, u64>,
}

impl Tally {
    fn add(&mut self, f: &Finding) {
        let (n, preexisting) = self.findings.entry((f.rule, f.severity)).or_default();
        *n += 1;
        if let Some(was) = f.preexisting {
            *preexisting.get_or_insert(0) += u64::from(was);
        }
    }

    fn findings(&self) -> Vec<RuleCount> {
        self.findings
            .iter()
            .map(|(&(rule, severity), &(findings, preexisting))| RuleCount {
                rule,
                severity,
                findings,
                preexisting,
            })
            .collect()
    }

    fn alone(&self) -> Vec<Alone> {
        self.alone
            .iter()
            .map(|(&rule, &units)| Alone { rule, units })
            .collect()
    }
}

/// Aggregate `rows` and the `skipped` parentless commits under the policy `under` names.
pub fn summarize(rows: &[Row], skipped: u64, under: Under) -> Summary {
    let block_at = under.block_at;
    let units = rows.len() as u64;
    let silent = rows.iter().filter(|r| r.findings.is_empty()).count() as u64;
    let mut blocking = 0;
    let mut all = Tally::default();
    let mut by_language: BTreeMap<&'static str, Tally> = BTreeMap::new();
    for row in rows {
        for f in &row.findings {
            all.add(f);
            by_language.entry(language(&f.path)).or_default().add(f);
        }
        let blockers: Vec<&Finding> = row
            .findings
            .iter()
            .filter(|f| f.severity <= block_at)
            .collect();
        blocking += u64::from(!blockers.is_empty());
        let rules: BTreeSet<Rule> = blockers.iter().map(|f| f.rule).collect();
        let [rule] = rules.into_iter().collect::<Vec<_>>()[..] else {
            continue;
        };
        *all.alone.entry(rule).or_default() += 1;
        let languages: BTreeSet<&str> = blockers.iter().map(|f| language(&f.path)).collect();
        for l in languages {
            *by_language
                .entry(l)
                .or_default()
                .alone
                .entry(rule)
                .or_default() += 1;
        }
    }
    Summary {
        v: 1,
        command: "backtest",
        policy: under.policy,
        policy_from: under.from,
        block_at,
        units,
        skipped,
        blocking,
        blocking_per_mille: per_mille(blocking, units),
        silent,
        silent_per_mille: per_mille(silent, units),
        findings: all.findings(),
        blocked_alone: all.alone(),
        languages: by_language
            .into_iter()
            .map(|(language, t)| ByLanguage {
                language,
                findings: t.findings(),
                blocked_alone: t.alone(),
            })
            .collect(),
        rows_written: None,
        does_not_say: DOES_NOT_SAY,
    }
}

fn language(path: &str) -> &'static str {
    Lang::of(path).map_or("other", Lang::name)
}

fn per_mille(n: u64, of: u64) -> u64 {
    n.saturating_mul(1000).checked_div(of).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use jtac_core::GitOid;

    fn f(rule: Rule, severity: Severity, path: &str, preexisting: Option<bool>) -> Finding {
        Finding {
            rule,
            severity,
            path: path.into(),
            line: 0,
            message: "m".into(),
            preexisting,
        }
    }

    fn under(block_at: Severity) -> Under {
        Under {
            block_at,
            policy: Sha256Digest::of("policy"),
            from: PolicyFrom::NewestCommit,
        }
    }

    fn row(findings: Vec<Finding>) -> Row {
        Row {
            commit: "sha1:0000000000000000000000000000000000000001"
                .parse::<GitOid>()
                .unwrap(),
            parents: 1,
            paths: 1,
            added_lines: 1,
            tiers: BTreeSet::new(),
            blocks: false,
            findings,
        }
    }

    fn count(rule: Rule, severity: Severity, findings: u64, pre: Option<u64>) -> RuleCount {
        RuleCount {
            rule,
            severity,
            findings,
            preexisting: pre,
        }
    }

    fn alone(rule: Rule, units: u64) -> Alone {
        Alone { rule, units }
    }

    fn rows() -> Vec<Row> {
        use Rule::*;
        use Severity::*;
        vec![
            row(vec![]),
            row(vec![f(Test001, Blocker, "src/a.rs", None)]),
            row(vec![
                f(Size001, Major, "src/big.rs", Some(true)),
                f(Size001, Major, "app/x.py", Some(false)),
                f(Cmt001, Minor, "src/big.rs", Some(true)),
            ]),
            row(vec![
                f(Prot001, Major, ".github/ci.yml", None),
                f(Test001, Blocker, "src/a.rs", None),
            ]),
            row(vec![f(Add001, Minor, "", None)]),
        ]
    }

    #[test]
    fn units_shares_and_findings_by_rule_and_severity_carry_the_preexisting_count() {
        let s = summarize(&rows(), 2, under(Severity::Major));
        assert_eq!((s.units, s.skipped), (5, 2));
        assert_eq!(
            (s.policy, s.policy_from, s.block_at),
            (
                Sha256Digest::of("policy"),
                PolicyFrom::NewestCommit,
                Severity::Major
            )
        );
        assert_eq!((s.blocking, s.blocking_per_mille), (3, 600));
        assert_eq!((s.silent, s.silent_per_mille), (1, 200));
        assert_eq!(
            s.findings,
            [
                count(Rule::Prot001, Severity::Major, 1, None),
                count(Rule::Test001, Severity::Blocker, 2, None),
                count(Rule::Size001, Severity::Major, 2, Some(1)),
                count(Rule::Cmt001, Severity::Minor, 1, Some(1)),
                count(Rule::Add001, Severity::Minor, 1, None),
            ]
        );
        assert_eq!(s.rows_written, None);
        assert_eq!(s.does_not_say, DOES_NOT_SAY);
    }

    #[test]
    fn a_unit_is_blocked_by_a_rule_alone_only_when_no_other_rule_blocks_it() {
        let s = summarize(&rows(), 0, under(Severity::Major));
        assert_eq!(
            s.blocked_alone,
            [alone(Rule::Test001, 1), alone(Rule::Size001, 1)],
            "the unit with JT-PROT-001 and JT-TEST-001 counts under neither"
        );
        let strict = summarize(&rows(), 0, under(Severity::Blocker));
        assert_eq!(strict.blocked_alone, [alone(Rule::Test001, 2)]);
        assert_eq!(strict.blocking, 2);
    }

    #[test]
    fn the_language_split_keys_each_finding_by_its_path_and_other_for_none() {
        let s = summarize(&rows(), 0, under(Severity::Major));
        let names: Vec<&str> = s.languages.iter().map(|l| l.language).collect();
        assert_eq!(names, ["other", "python", "rust"]);
        let python = &s.languages[1];
        assert_eq!(
            python.findings,
            [count(Rule::Size001, Severity::Major, 1, Some(0))]
        );
        assert_eq!(python.blocked_alone, [alone(Rule::Size001, 1)]);
        let rust = &s.languages[2];
        assert_eq!(
            rust.findings,
            [
                count(Rule::Test001, Severity::Blocker, 2, None),
                count(Rule::Size001, Severity::Major, 1, Some(1)),
                count(Rule::Cmt001, Severity::Minor, 1, Some(1)),
            ]
        );
        assert_eq!(
            rust.blocked_alone,
            [alone(Rule::Test001, 1), alone(Rule::Size001, 1)]
        );
        let other = &s.languages[0];
        assert_eq!(
            other.findings,
            [
                count(Rule::Prot001, Severity::Major, 1, None),
                count(Rule::Add001, Severity::Minor, 1, None),
            ]
        );
        assert!(other.blocked_alone.is_empty());
    }

    #[test]
    fn zero_units_give_zero_shares_and_empty_breakdowns() {
        let s = summarize(&[], 1, under(Severity::Major));
        assert_eq!(
            (s.units, s.skipped, s.blocking_per_mille, s.silent_per_mille),
            (0, 1, 0, 0)
        );
        assert!(s.findings.is_empty() && s.blocked_alone.is_empty() && s.languages.is_empty());
    }
}
