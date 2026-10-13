//! Why a policy was refused. Display is the WHAT line, as `jtac_core::RootError` does;
//! `three_parts` adds the why and the next command for the CLI (DESIGN.md section 12).
//! Text copied from the policy file arrives through `echo::shown`, never raw.

use jtac_core::canonical::MAX_SAFE_INTEGER;

use crate::POLICY_PATH;
use crate::schema::{DEFAULT_TIMEOUT_SECS, JTAC_DIR_GLOB};

/// A policy that does not parse or does not validate. Every one denies; nothing falls back to
/// defaults (DESIGN.md section 10.2).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PolicyError {
    /// Not TOML, not this shape, an unknown key, a float, or a negative integer.
    #[error("{}: {detail}", place(.position))]
    Parse {
        /// 1-based line and column (in chars) of the offending text, when the parser knows.
        position: Option<(u32, u32)>,
        /// The parser's message.
        detail: String,
    },
    /// A schema version this build has no reader for.
    #[error("{path} has version {found}; this build reads policy version 1", path = POLICY_PATH)]
    Version {
        /// The version in the file.
        found: u32,
    },
    /// `protected.paths` without `.jtac/**`.
    #[error("protected.paths does not include `{glob}`", glob = JTAC_DIR_GLOB)]
    JtacUnprotected,
    /// A check id outside `[a-z0-9_-]{1,64}`.
    #[error("check id `{id}` is not 1 to 64 bytes of a-z, 0-9, _ and -")]
    CheckId {
        /// The id as written.
        id: String,
    },
    /// A check whose argv has no program.
    #[error("checks.{id}.run names no program")]
    EmptyArgv {
        /// The check.
        id: String,
    },
    /// A check with `timeout_secs = 0`.
    #[error("checks.{id}.timeout_secs is 0")]
    Timeout {
        /// The check.
        id: String,
    },
    /// An integer past 2^53-1.
    #[error("{key} is larger than 2^53-1")]
    TooLarge {
        /// Dotted key, as `integrity.max_file_lines`.
        key: String,
    },
    /// A tier name outside `[a-z0-9_-]{1,64}`.
    #[error("tier name `{name}` is not 1 to 64 bytes of a-z, 0-9, _ and -")]
    TierName {
        /// The name as written.
        name: String,
    },
    /// A path or branch glob that does not compile or can never match.
    #[error("{key} pattern `{pattern}` is refused: {detail}")]
    Glob {
        /// Dotted key with index, as `protected.paths[2]`.
        key: String,
        /// The pattern as written.
        pattern: String,
        /// Why it is refused.
        detail: String,
    },
}

/// The tail every `how` shares: the edit leaves the tree dirty, and only `--preview` runs there.
pub(crate) const NEXT: &str = ", then run: jtac check --preview";

fn place(position: &Option<(u32, u32)>) -> String {
    match position {
        Some((line, column)) => format!("{POLICY_PATH} line {line}, column {column}"),
        None => format!("in {POLICY_PATH}"),
    }
}

impl PolicyError {
    /// What happened, why the rule exists, and the next command, for the CLI to print.
    pub fn three_parts(&self) -> [String; 3] {
        [self.to_string(), self.why().to_owned(), self.how()]
    }

    fn why(&self) -> &'static str {
        match self {
            Self::Parse { .. } => {
                "A policy that does not parse denies and never falls back to defaults, and an \
                 unknown key is an error so a typo cannot disable a rule. Do not delete the file \
                 to get past this: no policy file is a refusal too."
            }
            Self::Version { found: 0 } => {
                "Policy versions start at 1, and a version this build has no reader for is \
                 refused, never guessed at."
            }
            Self::Version { .. } => {
                "Policy schemas are versioned and a shipped version is never edited, so a \
                 version this build has no reader for is refused, never guessed at. Do not \
                 rewrite the number to get past this: the keys were written for that version."
            }
            Self::JtacUnprotected => {
                "`.jtac/` holds the policy and its trust files, so a policy file cannot \
                 unprotect itself."
            }
            Self::CheckId { .. } => {
                "A check id names node bodies and cache keys, so it has one safe spelling."
            }
            Self::EmptyArgv { .. } => {
                "run is the argv a check spawns, with no shell. With no program the check cannot \
                 run, and a check that did not run must never look like one that passed."
            }
            Self::Timeout { .. } => {
                "Every check runs under a timeout. Zero kills it before it starts, so it could \
                 never pass."
            }
            Self::TooLarge { .. } => {
                "The policy hash is over canonical JSON (RFC 8785), which writes every number as \
                 a double; a larger integer would hash differently in another implementation."
            }
            Self::TierName { .. } => {
                "A tier name is recorded in evidence, so it has the same safe spelling as a \
                 check id."
            }
            Self::Glob { .. } => {
                "A pattern that does not compile, or can never match a repository-relative path \
                 or branch, would silently guard nothing."
            }
        }
    }

    fn how(&self) -> String {
        let spelling = "1 to 64 bytes of [a-z0-9_-]";
        let fix = match self {
            Self::Parse {
                position: Some((line, column)),
                ..
            } => format!("edit {POLICY_PATH} line {line}, column {column}"),
            Self::Parse { position: None, .. } => format!("edit {POLICY_PATH}"),
            Self::Version { found: 0 } => format!("set version = 1 in {POLICY_PATH}"),
            Self::Version { found } => format!("install a jtac that reads policy version {found}"),
            Self::JtacUnprotected => {
                format!("add \"{JTAC_DIR_GLOB}\" to protected.paths in {POLICY_PATH}")
            }
            Self::CheckId { id } => {
                format!("rename [checks.{id}] in {POLICY_PATH} to {spelling}")
            }
            Self::EmptyArgv { id } => format!(
                "set run under [checks.{id}] in {POLICY_PATH} to the program and its arguments, \
                 as run = [\"cargo\", \"test\"]"
            ),
            Self::Timeout { id } => format!(
                "set timeout_secs = {DEFAULT_TIMEOUT_SECS} under [checks.{id}] in {POLICY_PATH}"
            ),
            Self::TooLarge { key } => {
                format!("set {key} to {MAX_SAFE_INTEGER} or less in {POLICY_PATH}")
            }
            Self::TierName { name } => {
                format!("rename [tiers.{name}] in {POLICY_PATH} to {spelling}")
            }
            Self::Glob { key, .. } => format!(
                "fix {key} in {POLICY_PATH}: globset syntax, relative to the repository root, \
                 / between directories, [*] for a literal *"
            ),
        };
        format!("{fix}{NEXT}")
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// Variant name, so tables assert the variant and not the text.
    pub(crate) fn kind(error: &PolicyError) -> &'static str {
        match error {
            PolicyError::Parse { .. } => "parse",
            PolicyError::Version { .. } => "version",
            PolicyError::JtacUnprotected => "jtac_unprotected",
            PolicyError::CheckId { .. } => "check_id",
            PolicyError::EmptyArgv { .. } => "empty_argv",
            PolicyError::Timeout { .. } => "timeout",
            PolicyError::TooLarge { .. } => "too_large",
            PolicyError::TierName { .. } => "tier_name",
            PolicyError::Glob { .. } => "glob",
        }
    }

    fn samples() -> Vec<PolicyError> {
        let text = |s: &str| s.to_owned();
        vec![
            PolicyError::Parse {
                position: Some((3, 1)),
                detail: text("unknown field `when`"),
            },
            PolicyError::Parse {
                position: None,
                detail: text("recursion limit"),
            },
            PolicyError::Version { found: 0 },
            PolicyError::Version { found: 2 },
            PolicyError::JtacUnprotected,
            PolicyError::CheckId { id: text("Fmt") },
            PolicyError::EmptyArgv { id: text("test") },
            PolicyError::Timeout { id: text("test") },
            PolicyError::TooLarge {
                key: text("integrity.max_file_lines"),
            },
            PolicyError::TierName { name: text("High") },
            PolicyError::Glob {
                key: text("protected.paths[0]"),
                pattern: text("["),
                detail: text("unclosed character class"),
            },
        ]
    }

    #[test]
    fn every_error_variant_renders_three_parts() {
        let samples = samples();
        let variants: std::collections::BTreeSet<_> = samples.iter().map(kind).collect();
        assert_eq!(variants.len(), 9, "a variant has no sample");
        for error in &samples {
            let [what, why, how] = error.three_parts();
            assert_eq!(what, error.to_string());
            assert!(!what.is_empty() && !why.is_empty(), "{error:?}");
            let fix = how.strip_suffix(NEXT).unwrap_or_default();
            assert!(
                !fix.trim().is_empty(),
                "{error:?}: no fix before the shared tail"
            );
            assert!(!how.contains('{'), "{error:?}: unfilled value in {how}");
        }
    }

    #[test]
    fn parse_names_the_position_only_when_known() {
        let [what, _, how] = samples()[0].three_parts();
        assert!(
            what.starts_with(".jtac/policy.toml line 3, column 1: "),
            "{what}"
        );
        assert!(
            how.starts_with("edit .jtac/policy.toml line 3, column 1"),
            "{how}"
        );
        let [what, _, how] = samples()[1].three_parts();
        assert_eq!(what, "in .jtac/policy.toml: recursion limit");
        assert_eq!(how, format!("edit .jtac/policy.toml{NEXT}"));
    }

    #[test]
    fn version_why_and_how_agree() {
        let table: &[(u32, &str, &str)] = &[
            (0, "start at 1", "set version = 1"),
            (
                2,
                "Do not rewrite the number",
                "install a jtac that reads policy version 2",
            ),
        ];
        for (found, why_has, how_starts) in table {
            let [_, why, how] = PolicyError::Version { found: *found }.three_parts();
            assert!(why.contains(why_has), "{found}: {why}");
            assert!(how.starts_with(how_starts), "{found}: {how}");
        }
        let [_, why, _] = PolicyError::Version { found: 0 }.three_parts();
        assert!(!why.contains("Do not rewrite"), "{why}");
    }

    #[test]
    fn rename_fix_states_the_spelling_not_a_guess() {
        let [_, _, how] = PolicyError::CheckId { id: "Fmt".into() }.three_parts();
        assert_eq!(
            how,
            format!(
                "rename [checks.Fmt] in .jtac/policy.toml to 1 to 64 bytes of [a-z0-9_-]{NEXT}"
            )
        );
    }
}
