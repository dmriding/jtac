//! `jtac init --ci github --read <id>=<workflow>[:<job>]` (DESIGN.md section 15.2, decision
//! 68): the checks the operator names as read from CI, in the policy init writes. Pure. Init
//! guesses none and never rewrites a policy someone wrote: a kept one that lacks a named
//! check is refused with the lines to add.

use jtac_core::Pin;
use jtac_policy::{Check, Detected, POLICY_PATH, Policy, PolicyError, Source, default_policy};

use crate::init::ask;
use crate::refuse::Refusal;

const SPELLING: &str = "`--read <id>=<workflow path>[:<job>]`, as `--read \
                        test=.github/workflows/ci.yml:test`";

/// One `--read`: the check and where CI runs it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Read {
    /// The check id.
    pub id: String,
    /// The definition file and the job.
    pub pin: Pin,
}

/// The flag's values: the id to the first `=`, the path to the first `:`, the job after it.
pub fn parse(values: &[String]) -> Result<Vec<Read>, Refusal> {
    let mut reads: Vec<Read> = Vec::new();
    for value in values {
        let (id, rest) = value
            .split_once('=')
            .ok_or_else(|| Refusal::init_read_spelling(value))?;
        let (workflow, job) = match rest.split_once(':') {
            Some((workflow, job)) => (workflow, Some(job.to_owned())),
            None => (rest, None),
        };
        if reads.iter().any(|r| r.id == id) {
            return Err(Refusal::init_read_spelling(value));
        }
        reads.push(Read {
            id: id.to_owned(),
            pin: Pin {
                workflow: workflow.to_owned(),
                job,
            },
        });
    }
    Ok(reads)
}

/// `policy` with each read as a required `ci` check, which makes it version 2; with none,
/// unchanged. A read the policy's own rules refuse is refused here in their words.
pub fn apply(policy: Policy, reads: &[Read]) -> Result<Policy, Refusal> {
    put(policy, reads).map_err(|e| Refusal::init_read(&e))
}

fn put(mut policy: Policy, reads: &[Read]) -> Result<Policy, PolicyError> {
    if reads.is_empty() {
        return Ok(policy);
    }
    policy.version = 2;
    for read in reads {
        let check = Check {
            source: Source::Ci(read.pin.clone()),
            required: true,
        };
        policy.checks.insert(read.id.clone(), check);
    }
    jtac_policy::validate(&policy)?;
    Ok(policy)
}

/// A kept policy holds every read as written, or the refusal that names the lines. A read
/// refused under the default policy is the flag's fault; one only the kept file refuses is
/// the file's, and each refusal names its own fix.
pub fn held(kept: &Policy, reads: &[Read]) -> Result<(), Refusal> {
    apply(default_policy(&Detected::default()), reads)?;
    put(kept.clone(), reads).map_err(|e| Refusal::init_read_file(&e))?;
    let lacking: Vec<&Read> = reads
        .iter()
        .filter(|r| kept.checks.get(&r.id).and_then(Check::pin) != Some(&r.pin))
        .collect();
    if lacking.is_empty() {
        Ok(())
    } else {
        Err(Refusal::init_read_kept(&lacking))
    }
}

/// The check as the policy file spells it.
pub fn lines(read: &Read) -> String {
    let basic = |s: &str| s.replace('\\', "\\\\").replace('"', "\\\"");
    let job =
        (read.pin.job.as_ref()).map_or_else(String::new, |j| format!(", job = \"{}\"", basic(j)));
    format!(
        "[checks.{}]\nci = {{ workflow = \"{}\"{job} }}\n",
        read.id,
        basic(&read.pin.workflow)
    )
}

impl Refusal {
    /// A `--read` value with no `=`, no id or no path, or an id named twice.
    pub fn init_read_spelling(value: &str) -> Self {
        ask(
            format!("`--read {value}` does not name one check, once, and its workflow."),
            "init guesses no check: each one read from CI is named with the definition file \
             that runs it, and the job where one job is the result, so the policy pins what \
             the merge reads.",
            format!("Run `jtac init --ci github` with {SPELLING}, once for each check."),
        )
    }

    /// The policy's own rules refuse a `--read`.
    pub fn init_read(err: &PolicyError) -> Self {
        let [what, why, _] = err.three_parts();
        ask(
            format!("`--read` names a check the policy cannot hold: {what}."),
            why,
            format!(
                "Run `jtac init --ci github` with {SPELLING}: the id 1 to 64 bytes of \
                 [a-z0-9_-], the path under `.github/` from the repository root, the job as \
                 the forge reports it."
            ),
        )
    }

    /// The tree's own policy, and not the default one, refuses a `--read`.
    pub fn init_read_file(err: &PolicyError) -> Self {
        let [what, why, fix] = err.parts_in(POLICY_PATH);
        ask(
            format!("{POLICY_PATH} in the work tree cannot hold a check `--read` names: {what}."),
            why,
            format!(
                "In the work tree, {fix}; then run `jtac init --ci github` again with the \
                 same flags."
            ),
        )
    }

    /// The tree's own policy does not read a `--read` check from CI.
    pub fn init_read_kept(lacking: &[&Read]) -> Self {
        let ids: Vec<String> = lacking.iter().map(|r| format!("`{}`", r.id)).collect();
        let lines: String = lacking.iter().map(|r| lines(r)).collect();
        ask(
            format!(
                "{POLICY_PATH} is in the work tree and does not read {} from CI as `--read` \
                 names it.",
                ids.join(", ")
            ),
            "init never rewrites a policy someone wrote: the file is the gate, and a change \
             to it is the operator's to make and a reviewer's to see.",
            format!(
                "Set `version = 2` in {POLICY_PATH}, replace each check below with these \
                 lines, then run `jtac init --ci github` again:\n{lines}"
            ),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read(id: &str, workflow: &str, job: Option<&str>) -> Read {
        Read {
            id: id.to_owned(),
            pin: Pin {
                workflow: workflow.to_owned(),
                job: job.map(str::to_owned),
            },
        }
    }

    fn values(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn the_flag_splits_at_the_first_equals_and_the_first_colon_and_names_an_id_once() {
        let ci = ".github/workflows/ci.yml";
        assert_eq!(
            parse(&values(&[
                "rust-test=.github/workflows/ci.yml:test (ubuntu, 1.80): a=b",
                "e2e=.github/workflows/ci.yml",
            ]))
            .unwrap(),
            [
                read("rust-test", ci, Some("test (ubuntu, 1.80): a=b")),
                read("e2e", ci, None),
            ]
        );
        assert_eq!(parse(&[]).unwrap(), []);
        for bad in [
            &["rust-test"][..],
            &["a=.github/workflows/ci.yml", "a=.github/workflows/e2e.yml"],
        ] {
            let r = parse(&values(bad)).unwrap_err();
            assert!(r.what.contains(bad[bad.len() - 1]), "{r:?}");
        }
    }

    #[test]
    fn a_read_makes_the_policy_version_2_and_the_policys_rules_refuse_a_bad_one() {
        let base = jtac_policy::parse("version = 1\n[checks.test]\nrun = [\"t\"]\n").unwrap();
        assert_eq!(apply(base.clone(), &[]).unwrap(), base);
        let ci = ".github/workflows/ci.yml";
        let two = apply(
            base.clone(),
            &[read("test", ci, Some("test")), read("e2e", ci, None)],
        )
        .unwrap();
        assert_eq!(two.version, 2);
        let pins: Vec<_> = two
            .ci_checks()
            .map(|(id, p)| (id, p.job.as_deref()))
            .collect();
        assert_eq!(pins, [("e2e", None), ("test", Some("test"))]);
        assert!(two.checks.values().all(|c| c.required));
        for (bad, want) in [
            (read("Test", ci, None), "check id"),
            (read("", ci, None), "check id"),
            (read("test", "", None), "checks.test.ci.workflow"),
            (read("test", "ci.yml", None), "protected.paths"),
            (read("test", ".github/workflows/*.yml", None), "ci.workflow"),
            (read("test", ci, Some("")), "checks.test.ci.job"),
            (read("test", ci, Some("caf\u{e9}")), "checks.test.ci.job"),
        ] {
            let r = apply(base.clone(), std::slice::from_ref(&bad)).unwrap_err();
            assert!(r.what.contains(want), "{bad:?}: {r:?}");
        }
    }

    #[test]
    fn a_kept_policy_holds_the_read_as_written_or_is_refused_with_lines_that_parse_back() {
        let ci = ".github/workflows/ci.yml";
        let reads = [
            read("test", ci, Some("a \"b\" \\ c")),
            read("e2e", ci, None),
        ];
        let text = format!("version = 2\n{}{}", lines(&reads[0]), lines(&reads[1]));
        let kept = jtac_policy::parse(&text).unwrap();
        assert_eq!(apply(kept.clone(), &[]).unwrap().ci_checks().count(), 2);
        assert_eq!(kept.checks["test"].pin(), Some(&reads[0].pin));
        held(&kept, &reads).unwrap();
        let one = jtac_policy::parse("version = 1\n[checks.test]\nrun = [\"t\"]\n").unwrap();
        let r = held(&one, &reads).unwrap_err();
        assert!(r.what.contains("`test`, `e2e`"), "{r:?}");
        assert!(r.how.contains(&lines(&reads[0])) && r.how.contains("version = 2"));
        let other = [read("test", ci, Some("other"))];
        let r = held(&kept, &other).unwrap_err();
        assert!(
            r.what.contains("`test`") && !r.what.contains("e2e"),
            "{r:?}"
        );
        let r = held(&kept, &[read("x", "ci.yml", None)]).unwrap_err();
        assert!(r.what.contains("protected.paths"), "{r:?}");
        assert!(
            r.how.contains("the path under `.github/`"),
            "the flag's: {r:?}"
        );
        let open =
            jtac_policy::parse("version = 1\n[protected]\npaths = [\".jtac/**\"]\n").unwrap();
        let r = held(&open, &[read("test", ci, None)]).unwrap_err();
        assert!(
            r.what.starts_with(".jtac/policy.toml in the work tree"),
            "{r:?}"
        );
        assert!(
            r.how.contains(
                "add the path `.github/workflows/ci.yml` to protected.paths in .jtac/policy.toml"
            ) && !r.how.contains("--read"),
            "the file's: {r:?}"
        );
    }

    #[test]
    fn every_read_refusal_has_three_parts_and_names_the_next_command() {
        let pin = PolicyError::CheckId { id: "X".into() };
        let lacking = read("test", ".github/workflows/ci.yml", None);
        for r in [
            Refusal::init_read_spelling("x"),
            Refusal::init_read(&pin),
            Refusal::init_read_file(&pin),
            Refusal::init_read_kept(&[&lacking]),
        ] {
            assert!(!r.what.is_empty() && !r.why.is_empty(), "{r:?}");
            assert!(r.how.contains("`jtac init --ci github`"), "{r:?}");
            assert!(r.text().contains("\nnegotiation: "), "{r:?}");
        }
    }
}
