//! Each verdict reason as the three parts a person or a model acts on (DESIGN.md section 12):
//! a finding's why and fix come from the rule table with its path filled in, a check's from
//! the evidence rules.

use jtac_checks::diff::Rule;
use jtac_core::{Body, Observed, ObservedCheck, Outcome, Reason, Severity};
use serde::{Deserialize, Serialize};

use crate::judge::Judged;
use crate::output::plain;

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
                        doubled, unnamed by the policy, or from another source than it names is \
                        evidence the gate cannot vouch for.";

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
        Reason::UncountedCheck { id } => other(
            id,
            "has a result from another source, pin or commit than the policy names",
        ),
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

/// A check that is not `pass`, explained from the node the verdict counted for it: the
/// run's, or for a check read from CI the forge's word, never another node of the same id
/// (`a_failed_check_is_explained_from_the_node_the_verdict_counted`).
fn check(p: &Judged, id: &str, outcome: Outcome) -> Explained {
    let spec = p.facts.checks.iter().find(|c| c.id == id);
    let counts = |source| spec.is_some_and(|s| s.counts(source, &p.facts.commit));
    let about = About::Check {
        id: id.to_owned(),
        outcome,
    };
    let counted = p.nodes.iter().find_map(|n| match &n.body {
        Body::Check(c) if c.id == id && counts(None) => Some(Ok(c)),
        Body::Observed(c) if c.id == id && counts(Some(&c.observed)) => Some(Err(c)),
        _ => None,
    });
    let body = match counted {
        None => return other(id, "has no result for this tree and policy"),
        Some(Err(read)) => return observed(p, about, read),
        Some(Ok(body)) => body,
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

/// Where a result was read from, every word plain: the job when policy names one, the
/// definition file, the run and its attempt.
pub fn source(o: &Observed) -> String {
    let job = (o.job.as_deref()).map_or(String::new(), |j| format!("job {} of ", plain(j)));
    format!(
        "{job}{}, run {} attempt {}",
        plain(&o.workflow),
        o.run,
        o.attempt
    )
}

// A check read from CI. Nothing local reruns it: the fix is on the forge or in a new pull
// request, and the waiver is how a person ships over it (section 15.2).
fn observed(p: &Judged, about: About, read: &ObservedCheck) -> Explained {
    let o = &read.observed;
    Explained {
        about,
        what: format!(
            "check `{}` {}: CI concluded `{}` ({})",
            plain(&read.id),
            read.outcome,
            plain(&o.conclusion),
            source(o)
        ),
        why: NOT_PASS.to_owned(),
        how: format!(
            "Read run {} on the forge. Re-run it there when the failure is not the change's, \
             and the merge job reads the new attempt; else fix it in a new pull request. To \
             ship over it, `jtac waive {} --ci --reason <why>` records why.",
            o.run,
            p.head.commit.hex()
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

    const WORKFLOW: &str = ".github/workflows/ci.yml";

    fn oid(last: char) -> jtac_core::GitOid {
        format!("sha1:{}{last}", "0".repeat(39)).parse().unwrap()
    }

    fn ran(id: &str, exit: i32, outcome: Outcome) -> Body {
        Body::Check(jtac_core::CheckBody {
            id: id.into(),
            argv: vec!["cargo".into(), id.into()],
            exit,
            outcome,
            ..crate::judge::integrity_body(None, 0)
        })
    }

    fn read(id: &str, word: &str) -> Body {
        let observed = Observed {
            forge: jtac_core::Forge::Github,
            workflow: WORKFLOW.into(),
            blob: oid('b'),
            job: Some("unit\ntests".into()),
            commit: oid('1'),
            run: 1234,
            attempt: 2,
            conclusion: word.into(),
        };
        Body::Observed(ObservedCheck {
            id: id.into(),
            outcome: observed.forge.outcome(word),
            observed,
        })
    }

    /// Facts with `fmt` run and `test` read from CI, judged over `bodies`.
    fn judged(bodies: Vec<Body>) -> Judged {
        let spec = |id: &str, pin: Option<jtac_core::Pin>| jtac_core::CheckSpec {
            id: id.into(),
            required: true,
            pin,
        };
        let pin = jtac_core::Pin {
            workflow: WORKFLOW.into(),
            job: Some("unit\ntests".into()),
        };
        let facts = jtac_core::Facts {
            commit: oid('1'),
            tree: oid('2'),
            policy: jtac_core::Sha256Digest::of(b"policy"),
            checks: vec![spec("fmt", None), spec("test", Some(pin))],
            block_at: Severity::Major,
            waivable: Vec::new(),
        };
        let now: jtac_core::Ts = "2026-10-10T18:30:00Z".parse().unwrap();
        let nodes: Vec<jtac_core::Node> = bodies
            .into_iter()
            .map(|body| jtac_core::Node {
                tree: facts.tree,
                policy: facts.policy,
                ts: now,
                body,
            })
            .collect();
        let head = crate::repo::Head {
            commit: facts.commit,
            tree: facts.tree,
        };
        Judged {
            head,
            base: crate::repo::Base {
                commit: "0".repeat(40),
                from: "--base".into(),
            },
            verdict: jtac_core::gate(&facts, &nodes, now),
            facts,
            unread: Vec::new(),
            now,
            nodes,
            levers: Vec::new(),
            captures: Vec::new(),
            integrity_error: None,
            report: None,
        }
    }

    fn explained(p: &Judged) -> Vec<Explained> {
        p.verdict.reasons.iter().map(|r| explain(r, p)).collect()
    }

    /// Issue 161 note 4: a check read from CI is explained by the forge's word and never
    /// by a run's node of the same id, and a run check never by an observed one.
    #[test]
    fn a_failed_check_is_explained_from_the_node_the_verdict_counted() {
        let stray_run = ran("test", 7, Outcome::Fail);
        let p = judged(vec![
            ran("fmt", 0, Outcome::Pass),
            stray_run,
            read("test", "failure"),
        ]);
        let texts = explained(&p);
        let failed = (texts.iter())
            .find(|e| e.what.starts_with("check `test` fail"))
            .unwrap_or_else(|| panic!("{texts:?}"));
        assert_eq!(
            failed.what,
            "check `test` fail: CI concluded `failure` (job unit tests of \
             .github/workflows/ci.yml, run 1234 attempt 2)"
        );
        assert!(
            failed.how.contains("Read run 1234 on the forge"),
            "{failed:?}"
        );
        let waive = format!("`jtac waive {} --ci --reason <why>`", p.head.commit.hex());
        assert!(failed.how.contains(&waive), "{failed:?}");
        assert!(
            texts.iter().all(|e| !e.what.contains("exit 7")),
            "{texts:?}"
        );

        let stray_read = read("fmt", "success");
        let p = judged(vec![
            ran("fmt", 3, Outcome::Fail),
            stray_read,
            read("test", "success"),
        ]);
        let texts = explained(&p);
        let failed = (texts.iter())
            .find(|e| e.what.starts_with("check `fmt` fail"))
            .unwrap_or_else(|| panic!("{texts:?}"));
        assert_eq!(failed.what, "check `fmt` fail, exit 3");
        assert!(failed.how.contains("`cargo fmt`"), "{failed:?}");
    }

    /// Issue 161 note 9, DESIGN.md section 12: every reason the verdict can give reads as
    /// three parts, and each evidence reason says which fault it is.
    #[test]
    fn every_reason_has_three_parts_and_says_which_fault_it_is() {
        let pass = || ran("fmt", 0, Outcome::Pass);
        let ok = || read("test", "success");
        let rows = [
            ("check `test` has no result", vec![pass()]),
            ("check `fmt` has two results", vec![pass(), pass(), ok()]),
            (
                "check `lint` is not a check the policy names",
                vec![pass(), ok(), ran("lint", 0, Outcome::Pass)],
            ),
            (
                "check `test` has a result from another source, pin or commit",
                vec![pass(), ok(), ran("test", 0, Outcome::Pass)],
            ),
            (
                "check `fmt` skipped, exit 0",
                vec![ran("fmt", 0, Outcome::Skipped), ok()],
            ),
            (
                "check `test` error: CI concluded `cancelled`",
                vec![pass(), read("test", "cancelled")],
            ),
        ];
        for (want, bodies) in rows {
            let p = judged(bodies);
            let texts = explained(&p);
            assert!(
                texts.iter().any(|e| e.what.starts_with(want)),
                "{want}: {texts:?}"
            );
            for e in &texts {
                assert!(!e.why.is_empty() && !e.how.is_empty(), "{want}: {e:?}");
                assert!(!e.what.chars().any(char::is_control), "{want}: {e:?}");
            }
        }
        let f = explain(
            &Reason::Finding {
                rule: "JT-PROT-001".into(),
                severity: Severity::Major,
                path: ".jtac/policy.toml".into(),
                line: 0,
                message: "changed".into(),
            },
            &judged(Vec::new()),
        );
        assert!(f.what.starts_with("JT-PROT-001 major"), "{f:?}");
        assert!(!f.why.is_empty() && !f.how.is_empty(), "{f:?}");
    }
}
