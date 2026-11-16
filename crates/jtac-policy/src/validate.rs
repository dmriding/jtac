//! Rules the types cannot express (DESIGN.md sections 9.1, 10.2 and 15.2). The order is fixed, so
//! a policy with several faults always reports the same one first.

use jtac_core::Pin;
use jtac_core::canonical::MAX_SAFE_INTEGER;

use crate::check::Source;
use crate::echo::shown;
use crate::error::PolicyError;
use crate::glob::{Matcher, not_a_file};
use crate::schema::{INTEGRITY_CHECK_ID, JTAC_DIR_GLOB, Policy};

/// Longest check id or tier name, in bytes.
pub(crate) const MAX_NAME_BYTES: usize = 64;

/// Whether `c` may appear in a check id or tier name.
pub(crate) fn name_char(c: char) -> bool {
    matches!(c, 'a'..='z' | '0'..='9' | '_' | '-')
}

fn is_name(s: &str) -> bool {
    !s.is_empty() && s.len() <= MAX_NAME_BYTES && s.chars().all(name_char)
}

/// Refuse what parses but cannot be enforced as written.
pub fn validate(p: &Policy) -> Result<(), PolicyError> {
    if !matches!(p.version, 1 | 2) {
        return Err(PolicyError::Version { found: p.version });
    }
    if !p.protected.paths.iter().any(|path| path == JTAC_DIR_GLOB) {
        return Err(PolicyError::JtacUnprotected);
    }
    for (id, check) in &p.checks {
        if !is_name(id) {
            return Err(PolicyError::CheckId { id: shown(id) });
        }
        if id == INTEGRITY_CHECK_ID {
            return Err(PolicyError::ReservedId { id: id.clone() });
        }
        let Source::Run { argv, timeout_secs } = &check.source else {
            continue;
        };
        let id = id.clone();
        if argv.first().is_none_or(String::is_empty) {
            return Err(PolicyError::EmptyArgv { id });
        }
        if *timeout_secs == 0 {
            return Err(PolicyError::Timeout { id });
        }
        safe_integer(format!("checks.{id}.timeout_secs"), Some(*timeout_secs))?;
    }
    let i = &p.integrity;
    safe_integer(
        "integrity.max_new_suppressions",
        Some(i.max_new_suppressions),
    )?;
    safe_integer("integrity.max_file_lines", Some(i.max_file_lines))?;
    safe_integer("integrity.max_function_lines", Some(i.max_function_lines))?;
    safe_integer("integrity.max_added_lines", i.max_added_lines)?;
    if let Some(name) = p.tiers.keys().find(|name| !is_name(name)) {
        return Err(PolicyError::TierName { name: shown(name) });
    }
    let protected = p.matchers()?.protected;
    (p.ci_checks()).try_for_each(|(id, pin)| pinned(p.version, id, pin, &protected))
}

/// Longest job name, in bytes.
const MAX_JOB_BYTES: usize = 255;

/// A `ci` check is version 2's, names one protected file in full, and a job a diff shows whole.
fn pinned(version: u32, id: &str, pin: &Pin, protected: &Matcher) -> Result<(), PolicyError> {
    if version < 2 {
        return Err(PolicyError::CiVersion { id: id.to_owned() });
    }
    let refused = |field: &str, value: &str, detail: &str| PolicyError::CiPin {
        key: format!("checks.{id}.ci.{field}"),
        value: shown(value),
        detail: detail.to_owned(),
    };
    if let Some(why) = not_a_file(&pin.workflow) {
        return Err(refused("workflow", &pin.workflow, why));
    }
    if let Some(job) = &pin.job
        && let Some(why) = not_a_job(job)
    {
        return Err(refused("job", job, why));
    }
    if !protected.is_match(&pin.workflow) {
        return Err(PolicyError::CiUnprotected {
            id: id.to_owned(),
            workflow: pin.workflow.clone(),
        });
    }
    Ok(())
}

fn not_a_job(job: &str) -> Option<&'static str> {
    if job.is_empty() {
        Some("an empty name names no job: leave `job` out to read the run's own conclusion")
    } else if job.len() > MAX_JOB_BYTES {
        Some("a job name is at most 255 bytes")
    } else if job.chars().any(|c| !(' '..='~').contains(&c)) {
        Some("printable ASCII only, so a diff shows every byte: rename the job")
    } else if job.starts_with(' ') || job.ends_with(' ') {
        Some("a job name does not start or end with a space")
    } else {
        None
    }
}

fn safe_integer(key: impl Into<String>, value: Option<u64>) -> Result<(), PolicyError> {
    match value {
        Some(v) if v > MAX_SAFE_INTEGER => Err(PolicyError::TooLarge { key: key.into() }),
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse::parse;

    #[test]
    fn validate_refuses_a_policy_without_jtac_protected() {
        let mut policy = parse("version = 1").unwrap();
        assert_eq!(validate(&policy), Ok(()));
        for paths in [vec![], vec![".jtac/*".to_owned()], vec!["**".to_owned()]] {
            policy.protected.paths = paths;
            assert_eq!(validate(&policy), Err(PolicyError::JtacUnprotected));
        }
    }

    #[test]
    fn too_large_names_every_guarded_key() {
        type Set = fn(&mut Policy, u64);
        let big = MAX_SAFE_INTEGER + 1;
        let table: &[(Set, &str)] = &[
            (
                |p, v| p.integrity.max_new_suppressions = v,
                "integrity.max_new_suppressions",
            ),
            (
                |p, v| p.integrity.max_file_lines = v,
                "integrity.max_file_lines",
            ),
            (
                |p, v| p.integrity.max_function_lines = v,
                "integrity.max_function_lines",
            ),
            (
                |p, v| p.integrity.max_added_lines = Some(v),
                "integrity.max_added_lines",
            ),
            (
                |p, v| {
                    let Source::Run { timeout_secs, .. } =
                        &mut p.checks.get_mut("fmt").unwrap().source
                    else {
                        panic!("fmt is a run check");
                    };
                    *timeout_secs = v;
                },
                "checks.fmt.timeout_secs",
            ),
        ];
        for (set, key) in table {
            let mut policy = parse("version = 1\n[checks.fmt]\nrun = [\"x\"]").unwrap();
            set(&mut policy, MAX_SAFE_INTEGER);
            assert_eq!(validate(&policy), Ok(()), "{key} at the limit");
            set(&mut policy, big);
            let want = Err(PolicyError::TooLarge {
                key: (*key).to_owned(),
            });
            assert_eq!(validate(&policy), want);
        }
    }

    #[test]
    fn name_spelling_table() {
        let table: &[(&str, bool)] = &[
            ("", false),
            ("Fmt", false),
            ("a/b", false),
            ("a.b", false),
            ("a b", false),
            ("ünit", false),
            ("fmt\n", false),
            (&"x".repeat(MAX_NAME_BYTES + 1), false),
            ("fmt", true),
            ("rust-fmt", true),
            ("py_test2", true),
            (&"x".repeat(MAX_NAME_BYTES), true),
        ];
        for (name, ok) in table {
            assert_eq!(is_name(name), *ok, "{name:?}");
        }
    }
}
