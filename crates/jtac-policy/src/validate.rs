//! Rules the types cannot express (DESIGN.md sections 9.1 and 10.2). The order is fixed, so a
//! policy with several faults always reports the same one first.

use jtac_core::canonical::MAX_SAFE_INTEGER;

use crate::echo::shown;
use crate::error::PolicyError;
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
    if p.version != 1 {
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
        let id = id.clone();
        if check.run.first().is_none_or(String::is_empty) {
            return Err(PolicyError::EmptyArgv { id });
        }
        if check.timeout_secs == 0 {
            return Err(PolicyError::Timeout { id });
        }
        safe_integer(
            format!("checks.{id}.timeout_secs"),
            Some(check.timeout_secs),
        )?;
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
    p.matchers().map(drop)
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
                |p, v| p.checks.get_mut("fmt").unwrap().timeout_secs = v,
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
