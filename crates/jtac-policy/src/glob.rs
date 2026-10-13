//! Path and branch globs (DESIGN.md section 10.2). `literal_separator` is off, so `**`, `*`,
//! and `?` all cross `/`: `release/*` matches `release/1.2/fix` and `*` matches every branch.
//! `backslash_escape` is pinned on so a pattern means the same thing on every platform.
//! Inputs are repository-relative with `/`; the caller normalizes.

use std::collections::BTreeMap;

use globset::{GlobBuilder, GlobSet, GlobSetBuilder};

use crate::echo::shown;
use crate::error::PolicyError;
use crate::schema::Policy;

/// A compiled list of globs. Matches when any one does; an empty list matches nothing.
#[derive(Clone, Debug)]
pub struct Matcher(GlobSet);

impl Matcher {
    /// Compile `patterns`. `key` names them in an error, as `protected.paths`.
    pub fn new(key: &str, patterns: &[String]) -> Result<Self, PolicyError> {
        let mut set = GlobSetBuilder::new();
        for (index, pattern) in patterns.iter().enumerate() {
            let glob = GlobBuilder::new(pattern)
                .literal_separator(false)
                .backslash_escape(true)
                .build()
                .map_err(|e| glob_error(format!("{key}[{index}]"), pattern, e.kind()))?;
            set.add(glob);
        }
        set.build()
            .map(Self)
            .map_err(|e| glob_error(key.to_owned(), &patterns.join(" "), e.kind()))
    }

    /// Whether any pattern matches `path`.
    pub fn is_match(&self, path: &str) -> bool {
        self.0.is_match(path)
    }
}

fn glob_error(key: String, pattern: &str, detail: impl ToString) -> PolicyError {
    PolicyError::Glob {
        key,
        pattern: shown(pattern),
        detail: shown(&detail.to_string()),
    }
}

/// Why `pattern` can never match a repository-relative path or a branch name, if it cannot.
fn unmatchable(pattern: &str) -> Option<&'static str> {
    if pattern.is_empty() {
        Some("an empty pattern matches nothing")
    } else if pattern.starts_with('/') || pattern.starts_with("./") {
        Some("paths are relative to the repository root, with no leading / or ./")
    } else if pattern.ends_with('/') {
        Some("a path never ends in /; use dir/** for everything under dir")
    } else if pattern.contains('\\') {
        Some(r"paths are matched with /, never \; write [*] for a literal *")
    } else {
        None
    }
}

/// `Matcher::new` for a policy list, refusing first what could never match.
fn policy_matcher(key: &str, patterns: &[String]) -> Result<Matcher, PolicyError> {
    for (index, pattern) in patterns.iter().enumerate() {
        if let Some(reason) = unmatchable(pattern) {
            return Err(glob_error(format!("{key}[{index}]"), pattern, reason));
        }
    }
    Matcher::new(key, patterns)
}

/// Every glob list in a policy, compiled.
#[derive(Clone, Debug)]
pub struct Matchers {
    /// `protected.paths`.
    pub protected: Matcher,
    /// `tiers.<name>.paths` by name.
    pub tiers: BTreeMap<String, Matcher>,
    /// `scope.paths`: where an agent is cleared to work. Never narrows what the gate checks.
    pub scope_paths: Matcher,
    /// `scope.branches`: where an agent is cleared to work. Never narrows what the gate checks.
    pub scope_branches: Matcher,
    /// `fences.protected_branches`.
    pub protected_branches: Matcher,
}

impl Policy {
    /// Compile every glob list. `validate` calls this, so a parsed policy always compiles.
    pub fn matchers(&self) -> Result<Matchers, PolicyError> {
        let tiers = self
            .tiers
            .iter()
            .map(|(name, tier)| {
                let matcher = policy_matcher(&format!("tiers.{name}.paths"), &tier.paths)?;
                Ok((name.clone(), matcher))
            })
            .collect::<Result<_, PolicyError>>()?;
        Ok(Matchers {
            protected: policy_matcher("protected.paths", &self.protected.paths)?,
            tiers,
            scope_paths: policy_matcher("scope.paths", &self.scope.paths)?,
            scope_branches: policy_matcher("scope.branches", &self.scope.branches)?,
            protected_branches: policy_matcher(
                "fences.protected_branches",
                &self.fences.protected_branches,
            )?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse::parse;
    use crate::schema::strings;

    #[test]
    fn glob_match_table() {
        let table: &[(&str, &str, bool)] = &[
            ("**/auth/**", "src/author/x.rs", false),
            ("**/auth/**", "auth", false),
            (".jtac/**", "x/.jtac/policy.toml", false),
            (".jtac/**", ".JTAC/policy.toml", false),
            ("clippy.toml", "sub/clippy.toml", false),
            ("main", "maint", false),
            ("main", "feature/main", false),
            ("release/*", "releases/1", false),
            ("**/auth/**", "src/auth/x.rs", true),
            ("**/auth/**", "auth/x.rs", true),
            (".jtac/**", ".jtac/policy.toml", true),
            (".jtac/**", ".jtac/nested/signers", true),
            ("release/*", "release/1.2", true),
            ("release/*", "release/1.2/fix", true),
            ("*", "feature/x", true),
            ("**", "a/b/c", true),
            ("main", "main", true),
            ("[*]", "*", true),
        ];
        for (pattern, path, want) in table {
            let matcher = Matcher::new("t", &strings(&[pattern])).unwrap();
            assert_eq!(matcher.is_match(path), *want, "{pattern} vs {path}");
        }
    }

    #[test]
    fn backslash_is_an_escape_on_every_platform() {
        let matcher = Matcher::new("t", &strings(&[r"a\*b"])).unwrap();
        assert!(matcher.is_match("a*b"));
        assert!(!matcher.is_match("axb"));
    }

    #[test]
    fn unmatchable_globs_are_refused() {
        let table = [
            "", "/src/**", "/", "./src/**", "./", "src/", "**/", r"src\**", r"a\*b", r"\",
        ];
        let sections = [
            "[protected]\npaths",
            "[tiers.high]\npaths",
            "[scope]\npaths",
            "[scope]\nbranches",
            "[fences]\nprotected_branches",
        ];
        for section in sections {
            for pattern in table {
                let text = format!("version = 1\n{section} = ['ok', '{pattern}']");
                let err = parse(&text).unwrap_err();
                let PolicyError::Glob { ref key, .. } = err else {
                    panic!("{text}: {err:?}")
                };
                assert!(key.ends_with("[1]"), "{text}: {key}");
            }
        }
    }

    #[test]
    fn empty_list_matches_nothing() {
        assert!(!Matcher::new("t", &[]).unwrap().is_match("a"));
    }

    #[test]
    fn bad_glob_error_names_key_index_and_pattern() {
        let err = Matcher::new("tiers.high.paths", &strings(&["ok/**", "["])).unwrap_err();
        let PolicyError::Glob { key, pattern, .. } = err else {
            panic!("{err:?}")
        };
        assert_eq!(
            (key.as_str(), pattern.as_str()),
            ("tiers.high.paths[1]", "[")
        );
    }

    #[test]
    fn oversized_glob_set_is_refused_not_dropped() {
        let huge = "?".repeat(200_000);
        let err = Matcher::new("scope.paths", &[huge]).unwrap_err();
        assert!(matches!(err, PolicyError::Glob { ref key, .. } if key == "scope.paths"));
    }
}
