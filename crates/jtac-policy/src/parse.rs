//! Policy text to a validated, resolved `Policy` (DESIGN.md section 10). Any failure denies.

use crate::echo::shown;
use crate::error::PolicyError;
use crate::schema::{Policy, with_jtac_dir};
use crate::validate::validate;

/// Parse, resolve, and validate `text`, the contents of `.jtac/policy.toml`. `.jtac/**` is
/// added to `protected.paths` when the file leaves it out: a file cannot unprotect itself.
pub fn parse(text: &str) -> Result<Policy, PolicyError> {
    let mut policy: Policy = toml::from_str(text).map_err(|e| parse_error(text, &e))?;
    policy.protected.paths = with_jtac_dir(&policy.protected.paths);
    validate(&policy)?;
    Ok(policy)
}

fn parse_error(text: &str, e: &toml::de::Error) -> PolicyError {
    PolicyError::Parse {
        position: e.span().and_then(|span| position(text, span.start)),
        detail: shown(e.message()),
    }
}

/// 1-based line and char column of byte `start`. None rather than a guess at 1:1.
fn position(text: &str, start: usize) -> Option<(u32, u32)> {
    let before = text.get(..start)?;
    let line_start = before.rfind('\n').map_or(0, |i| i + 1);
    let line = before.matches('\n').count() + 1;
    let column = before[line_start..].chars().count() + 1;
    Some((u32::try_from(line).ok()?, u32::try_from(column).ok()?))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::tests::kind;
    use crate::schema::JTAC_DIR_GLOB;

    const CHECK: &str = "[checks.fmt]\nrun = [\"cargo\", \"fmt\"]\n";

    fn refused(text: &str) -> &'static str {
        match parse(text) {
            Ok(p) => panic!("accepted {text:?} as {p:?}"),
            Err(e) => kind(&e),
        }
    }

    #[test]
    fn refusal_table() {
        let long_id = format!("version = 1\n[checks.{}]\nrun = [\"x\"]", "x".repeat(65));
        let table: &[(&str, &str)] = &[
            ("version = 1\n[checks.fmt]\nrun = \"cargo fmt\"", "parse"),
            (
                &format!("version = 1\n{CHECK}when = \"Cargo.toml\""),
                "parse",
            ),
            ("Version = 1", "parse"),
            ("version = 1\n[chekcs.fmt]\nrun = [\"x\"]", "parse"),
            ("version = 1\nshell = true", "parse"),
            ("version = 1\n[integrity]\nmax_new_supressions = 9", "parse"),
            ("version = 1\n[fences]\nno_verify = \"warn\"", "parse"),
            (
                "version = 1\n[fences]\nforce_push_protected = false",
                "parse",
            ),
            ("version = 1\n[integrity]\nblock_at = \"high\"", "parse"),
            ("version = 1\n[integrity]\nblock_at = \"Blocker\"", "parse"),
            ("version = 1.0", "parse"),
            ("version = \"1\"", "parse"),
            ("version = 4294967297", "parse"),
            ("[checks.fmt]\nrun = [\"x\"]", "parse"),
            ("", "parse"),
            ("version = 1\n[integrity]\nmax_file_lines = 800.0", "parse"),
            ("version = 1\n[integrity]\nmax_file_lines = inf", "parse"),
            (&format!("version = 1\n{CHECK}timeout_secs = nan"), "parse"),
            ("version = 1\n[checks.fmt]\nrun = [\"x\", 1.5]", "parse"),
            (
                "version = 1\n[integrity]\nmax_new_suppressions = -1",
                "parse",
            ),
            (&format!("version = 1\n{CHECK}required = \"yes\""), "parse"),
            ("version = 1\n[checks.fmt]\ntimeout_secs = 5", "parse"),
            ("version = 1\n[tiers.high]", "parse"),
            (&format!("version = 1\n{CHECK}{CHECK}"), "parse"),
            ("version = 1\nversion = 1", "parse"),
            (
                "version = 1\n[integrity]\nmax_file_lines = 9007199254740992",
                "too_large",
            ),
            (
                "version = 1\n[integrity]\nmax_added_lines = 9007199254740992",
                "too_large",
            ),
            (
                &format!("version = 1\n{CHECK}timeout_secs = 9007199254740992"),
                "too_large",
            ),
            ("version = 2", "version"),
            ("version = 0", "version"),
            ("version = 1\n[checks.fmt]\nrun = []", "empty_argv"),
            ("version = 1\n[checks.fmt]\nrun = [\"\"]", "empty_argv"),
            (
                "version = 1\n[checks.fmt]\nrun = [\"\", \"fmt\"]",
                "empty_argv",
            ),
            (&format!("version = 1\n{CHECK}timeout_secs = 0"), "timeout"),
            ("version = 1\n[checks.Fmt]\nrun = [\"x\"]", "check_id"),
            ("version = 1\n[checks.\"a/b\"]\nrun = [\"x\"]", "check_id"),
            ("version = 1\n[checks.\"\"]\nrun = [\"x\"]", "check_id"),
            (&long_id, "check_id"),
            ("version = 1\n[tiers.High]\npaths = [\"x\"]", "tier_name"),
            ("version = 1\n[protected]\npaths = [\"[\"]", "glob"),
            ("version = 1\n[tiers.high]\npaths = [\"[\"]", "glob"),
            ("version = 1\n[scope]\nbranches = [\"[\"]", "glob"),
            ("version = 1\n[scope]\npaths = [\"{a\"]", "glob"),
            (
                "version = 1\n[fences]\nprotected_branches = [\"[\"]",
                "glob",
            ),
        ];
        for (text, want) in table {
            assert_eq!(refused(text), *want, "{text:?}");
        }
    }

    #[test]
    fn boundary_values_are_accepted() {
        let text = format!(
            "version = 1\n{CHECK}required = false\ntimeout_secs = 9007199254740991\n\
             [checks.{}]\nrun = [\"x\"]\n[integrity]\nmax_added_lines = 9007199254740991",
            "x".repeat(64)
        );
        let policy = parse(&text).unwrap();
        assert_eq!(policy.integrity.max_added_lines, Some((1 << 53) - 1));
    }

    #[test]
    fn too_large_names_the_dotted_key() {
        let text = format!("version = 1\n{CHECK}timeout_secs = 9007199254740992");
        let key = "checks.fmt.timeout_secs".to_owned();
        assert_eq!(parse(&text), Err(PolicyError::TooLarge { key }));
    }

    #[test]
    fn parse_error_points_at_the_offending_line_and_column() {
        let text = format!("version = 1\n\n{CHECK}when = \"Cargo.toml\"\n");
        let Err(PolicyError::Parse { position, detail }) = parse(&text) else {
            panic!("not a parse error");
        };
        assert_eq!(position, Some((5, 1)), "{detail}");
        assert!(detail.contains("when"), "{detail}");
    }

    #[test]
    fn jtac_dir_is_protected_even_when_the_file_omits_it() {
        for paths in ["[]", "[\"src/**\"]", "[\"**\"]", "[\".jtac/*\"]"] {
            let policy = parse(&format!("version = 1\n[protected]\npaths = {paths}")).unwrap();
            assert_eq!(
                policy.protected.paths.last().unwrap(),
                JTAC_DIR_GLOB,
                "{paths}"
            );
            let protected = policy.matchers().unwrap().protected;
            assert!(protected.is_match(".jtac/nested/signers"), "{paths}");
        }
    }

    #[test]
    fn deep_dotted_key_reports_no_position_rather_than_one_one() {
        let text = format!("version = 1\n{} = 1", vec!["a"; 200].join("."));
        let Err(PolicyError::Parse { position, detail }) = parse(&text) else {
            panic!("not a parse error");
        };
        assert_eq!(position, None, "{detail}");
    }

    #[test]
    fn column_counts_chars_not_bytes() {
        let text = "version = 1\n[fences]\nprotected_branches = [\"\u{e9}\", 5]";
        let Err(PolicyError::Parse { position, .. }) = parse(text) else {
            panic!("not a parse error");
        };
        assert_eq!(position, Some((3, 28)));
    }

    #[test]
    fn echoed_values_are_capped_and_escaped() {
        let long = format!("A\u{202e}{}", "b".repeat(300));
        let want = format!(r"A\u{{202E}}{}…", "b".repeat(247));
        let texts = [
            format!("version = 1\n[checks.\"{long}\"]\nrun = [\"x\"]"),
            format!("version = 1\n[tiers.\"{long}\"]\npaths = [\"x\"]"),
            format!("version = 1\n[scope]\npaths = [\"{long}\"]"),
        ];
        let echoed: Vec<String> = texts
            .iter()
            .map(|text| match parse(text) {
                Err(PolicyError::CheckId { id }) => id,
                Err(PolicyError::TierName { name }) => name,
                Err(PolicyError::Glob { pattern, .. }) => pattern,
                other => panic!("{other:?}"),
            })
            .collect();
        assert_eq!(echoed, [want.clone(), want.clone(), want.clone()]);
        let [what, _, how] = parse(&texts[0]).unwrap_err().three_parts();
        assert_eq!(
            what,
            format!("check id `{want}` is not 1 to 64 bytes of a-z, 0-9, _ and -")
        );
        assert_eq!(
            how,
            format!(
                "rename the check id \"{want}\" in .jtac/policy.toml to 1 to 64 bytes of \
                 [a-z0-9_-], then run: jtac check --preview"
            )
        );
        let Err(PolicyError::Parse { detail, .. }) = parse("version = 1\n\"\u{202e}x\" = 1") else {
            panic!("not a parse error");
        };
        assert_eq!(
            detail,
            "unknown field `\\u{202E}x`, expected one of `version`, `checks`, `integrity`, \
             `protected`, `tiers`, `fences`, `scope`"
        );
    }

    #[test]
    fn glob_error_index_is_the_one_written() {
        let err = parse("version = 1\n[protected]\npaths = [\"ok\", \"[\"]").unwrap_err();
        assert!(matches!(err, PolicyError::Glob { ref key, .. } if key == "protected.paths[1]"));
    }

    #[test]
    fn parse_detail_is_one_line() {
        let Err(PolicyError::Parse { detail, .. }) = parse("version = 1\n\"a\\nb\" = 1") else {
            panic!("not a parse error");
        };
        assert!(!detail.contains('\n'), "{detail}");
    }

    #[test]
    fn jtac_dir_is_not_added_twice() {
        let policy = parse("version = 1\n[protected]\npaths = [\"a\", \".jtac/**\"]").unwrap();
        assert_eq!(policy.protected.paths, ["a", ".jtac/**"]);
    }
}
