//! Each verdict reason as the three parts a person or a model acts on (DESIGN.md section 12):
//! a finding's why and fix come from the rule table with its path filled in, a check's from
//! the evidence rules.

use jtac_checks::diff::Rule;
use jtac_core::{Body, Outcome, Reason, Severity};
use serde::{Deserialize, Serialize};

use crate::judge::Judged;

/// One reason, explained.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Explained {
    /// Which finding or check.
    #[serde(flatten)]
    pub about: About,
    /// What happened.
    pub what: String,
    /// Why the rule exists.
    pub why: String,
    /// The next step.
    pub how: String,
}

/// The evidence a reason points at.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum About {
    /// A finding at or above `block_at`.
    Finding {
        /// Rule id.
        rule: String,
        /// Severity.
        severity: Severity,
        /// Path.
        path: String,
        /// Line, zero when none.
        line: u64,
    },
    /// A check the verdict could not count as passed.
    Check {
        /// Check id.
        id: String,
        /// Its outcome, `error` for a missing, doubled, or unnamed check.
        outcome: Outcome,
    },
}

const NOT_PASS: &str = "A required check that is not `pass` fails the verdict. `skipped` and \
                        `error` are not `pass`: a check that did not run must never look like \
                        one that passed.";

const EVIDENCE: &str = "The verdict is computed from one node per policy check; a node missing, \
                        doubled, or unnamed by the policy is evidence the gate cannot vouch for.";

const AGAIN: &str = "Run `jtac check` again; if it repeats, report it with this output.";

/// `reason` in three parts, with the check's argv from `p.nodes`.
pub fn explain(reason: &Reason, p: &Judged) -> Explained {
    match reason {
        Reason::Finding {
            rule,
            severity,
            path,
            line,
            message,
        } => finding(rule, *severity, path, *line, message),
        Reason::CheckNotPass { id, outcome } => check(p, id, *outcome),
        Reason::MissingCheck { id } => other(id, "has no result for this tree and policy"),
        Reason::DuplicateCheck { id } => other(id, "has two results for this tree and policy"),
        Reason::UnknownCheck { id } => other(id, "is not a check the policy names"),
    }
}

/// A finding's line in the human output: rule, severity, place, message.
pub fn finding_line(
    rule: &str,
    severity: Severity,
    path: &str,
    line: u64,
    message: &str,
) -> String {
    let place = match (path.is_empty(), line) {
        (true, _) => String::new(),
        (false, 0) => format!(" {path}"),
        (false, n) => format!(" {path}:{n}"),
    };
    format!("{rule} {severity}{place}: {message}")
}

fn finding(rule: &str, severity: Severity, path: &str, line: u64, message: &str) -> Explained {
    let (why, how) = match Rule::from_id(rule) {
        Some(r) => (r.reason().to_owned(), r.fix().replace("<path>", path)),
        None => (
            "The rule is one this build's verdict knows and its table does not; the finding \
             blocks all the same."
                .to_owned(),
            AGAIN.to_owned(),
        ),
    };
    Explained {
        about: About::Finding {
            rule: rule.to_owned(),
            severity,
            path: path.to_owned(),
            line,
        },
        what: finding_line(rule, severity, path, line, message),
        why,
        how,
    }
}

fn check(p: &Judged, id: &str, outcome: Outcome) -> Explained {
    let body = p.nodes.iter().find_map(|n| match &n.body {
        Body::Check(c) if c.id == id => Some(c),
        _ => None,
    });
    let about = About::Check {
        id: id.to_owned(),
        outcome,
    };
    let Some(body) = body else {
        return other(id, "has no result for this tree and policy");
    };
    if body.argv.is_empty() {
        let error = p.integrity_error.as_deref().unwrap_or_default();
        return Explained {
            about,
            what: format!("check `{id}` {outcome}: {error}"),
            why: "The diff-integrity rules read every commit between the base and HEAD, and a \
                  run that could not read them is not a run that found nothing."
                .to_owned(),
            how: format!(
                "Run `git diff --stat {} {}` to see what git refuses, repair it (`git fsck`), \
                 and run `jtac check` again.",
                p.base.commit,
                p.head.commit.hex()
            ),
        };
    }
    Explained {
        about,
        what: format!("check `{id}` {outcome}, exit {}", body.exit),
        why: NOT_PASS.to_owned(),
        how: format!(
            "Run `{}` at the repository root and fix what it reports.",
            shown(&body.argv)
        ),
    }
}

fn other(id: &str, what: &str) -> Explained {
    Explained {
        about: About::Check {
            id: id.to_owned(),
            outcome: Outcome::Error,
        },
        what: format!("check `{id}` {what}"),
        why: EVIDENCE.to_owned(),
        how: AGAIN.to_owned(),
    }
}

/// An argv as a command line a person can paste: an argument outside the set no shell
/// expands is single-quoted, so the pasted line runs the argv that ran, not what a shell
/// makes of `$(...)`, `*`, or a backslash.
pub fn shown(argv: &[String]) -> String {
    let plain = |c: char| c.is_ascii_alphanumeric() || "_-./=:,@%+".contains(c);
    argv.iter()
        .map(|a| {
            if !a.is_empty() && a.chars().all(plain) {
                a.clone()
            } else {
                format!("'{}'", a.replace('\'', "'\\''"))
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn argv_is_shown_pasteable_and_finding_lines_omit_what_they_lack() {
        assert_eq!(shown(&["cargo".into(), "test".into()]), "cargo test");
        assert_eq!(
            shown(&["sh".into(), "-c".into(), "echo it's".into(), String::new()]),
            "sh -c 'echo it'\\''s' ''"
        );
        assert_eq!(
            shown(&[
                "t".into(),
                "$(x)".into(),
                "a*".into(),
                "b\\c".into(),
                "${IFS}".into()
            ]),
            "t '$(x)' 'a*' 'b\\c' '${IFS}'"
        );
        assert_eq!(
            shown(&["/usr/bin/x".into(), "--a=b,c".into()]),
            "/usr/bin/x --a=b,c"
        );
        assert_eq!(
            finding_line("JT-ADD-001", Severity::Minor, "", 0, "7 lines"),
            "JT-ADD-001 minor: 7 lines"
        );
        assert_eq!(
            finding_line(
                "JT-PROT-001",
                Severity::Major,
                ".jtac/policy.toml",
                0,
                "changed"
            ),
            "JT-PROT-001 major .jtac/policy.toml: changed"
        );
        assert_eq!(
            finding_line("JT-SUP-001", Severity::Blocker, "a.rs", 3, "x"),
            "JT-SUP-001 blocker a.rs:3: x"
        );
    }

    #[test]
    fn a_finding_reason_carries_its_own_message_and_fills_the_path_into_the_fix() {
        let got = finding(
            "JT-TEST-001",
            Severity::Blocker,
            "src/a.rs",
            0,
            "test `beta` removed",
        );
        assert_eq!(
            got.what,
            "JT-TEST-001 blocker src/a.rs: test `beta` removed"
        );
        assert!(got.how.contains("--path src/a.rs"), "{}", got.how);
        assert!(!got.how.contains("<path>"), "{}", got.how);
        let unknown = finding("JT-NEW-009", Severity::Major, "", 0, "x");
        assert!(unknown.how.contains("jtac check"));
    }
}
