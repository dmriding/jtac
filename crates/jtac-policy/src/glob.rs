//! Path and branch globs (DESIGN.md section 10.2). `literal_separator` is off, so `**`, `*`,
//! and `?` all cross `/`: `release/*` matches `release/1.2/fix` and `*` matches every branch.
//! `backslash_escape` is pinned on so a pattern means the same thing on every platform.
//! Inputs are repository-relative with `/`; the caller normalizes.

use std::collections::BTreeMap;

use globset::{GlobBuilder, GlobSet, GlobSetBuilder};

use crate::echo::shown;
use crate::error::PolicyError;
use crate::schema::{Policy, with_jtac_dir};

/// A compiled list of globs. Matches when any one does; an empty list matches nothing.
/// `?` matches one byte: one ASCII char, or part of any other. A non-ASCII path is matched
/// with `*`; `glob_match_table` pins the byte rule.
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

const EMPTY: &str = "an empty pattern matches nothing";
const LEADING: &str = "paths are relative to the repository root: no leading / or ./";
const TRAILING: &str = "a path never ends in /: write dir/** for everything under dir";
const BACKSLASH: &str = "paths are matched with /, never a backslash: write ? for a special char";
const ALTERNATION: &str = "alternation is not supported: write one pattern per branch";
const CLASS: &str = "character classes are not supported: the language is literals, ?, * and **";
const COMPONENT: &str = "a path has no empty, . or .. component";
const CHARACTER: &str = "printable ASCII only: match other characters with *";
const NEGATION: &str = "no leading !: there is no negation, write the pattern you want matched";
const EDGE: &str = "a pattern does not start or end with whitespace";
const BRANCH: &str = "git refuses this as a branch name: see git check-ref-format --branch";

/// Why `pattern` can never match a repository-relative path or a branch name, if it cannot
/// (DESIGN.md 10.2). Alternation and classes are refused so neither can smuggle in what the
/// raw-string checks refuse, as `[/]src/**` would a leading `/`.
fn unmatchable(pattern: &str) -> Option<&'static str> {
    let odd = |c: char| !(' '..='~').contains(&c);
    let edge = |c: Option<char>| c.is_some_and(char::is_whitespace);
    if pattern.is_empty() {
        Some(EMPTY)
    } else if pattern.starts_with('/') || pattern.starts_with("./") {
        Some(LEADING)
    } else if pattern.starts_with('!') {
        Some(NEGATION)
    } else if pattern.ends_with('/') {
        Some(TRAILING)
    } else if pattern.contains('\\') {
        Some(BACKSLASH)
    } else if pattern.contains(['{', '}']) {
        Some(ALTERNATION)
    } else if pattern.contains(['[', ']']) {
        Some(CLASS)
    } else if pattern
        .split('/')
        .any(|part| matches!(part, "" | "." | ".."))
    {
        Some(COMPONENT)
    } else if pattern.chars().any(odd) {
        Some(CHARACTER)
    } else if edge(pattern.chars().next()) || edge(pattern.chars().last()) {
        Some(EDGE)
    } else {
        None
    }
}

/// `unmatchable` plus the branch-name rules of `git check-ref-format --branch` that the path
/// rules do not already cover.
fn unmatchable_branch(pattern: &str) -> Option<&'static str> {
    let bad_part = |part: &str| part.starts_with('.') || part.ends_with(".lock");
    let refused = pattern == "HEAD"
        || pattern.starts_with('-')
        || pattern.ends_with('.')
        || pattern.contains("..")
        || pattern.contains([' ', '~', '^', ':'])
        || pattern.split('/').any(bad_part);
    unmatchable(pattern).or(refused.then_some(BRANCH))
}

/// `Matcher::new` for a policy list, refusing first what `refuse` says could never match.
fn policy_matcher(
    key: &str,
    patterns: &[String],
    refuse: fn(&str) -> Option<&'static str>,
) -> Result<Matcher, PolicyError> {
    for (index, pattern) in patterns.iter().enumerate() {
        if let Some(reason) = refuse(pattern) {
            return Err(glob_error(format!("{key}[{index}]"), pattern, reason));
        }
    }
    Matcher::new(key, patterns)
}

/// Every glob list in a policy, compiled.
#[derive(Clone, Debug)]
pub struct Matchers {
    /// `protected.paths`, always with `.jtac/**`.
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
    /// `.jtac/**` is protected here whatever the struct holds; `validate` reports its absence.
    pub fn matchers(&self) -> Result<Matchers, PolicyError> {
        let tiers = self
            .tiers
            .iter()
            .map(|(name, tier)| {
                let matcher =
                    policy_matcher(&format!("tiers.{name}.paths"), &tier.paths, unmatchable)?;
                Ok((name.clone(), matcher))
            })
            .collect::<Result<_, PolicyError>>()?;
        Ok(Matchers {
            protected: policy_matcher(
                "protected.paths",
                &with_jtac_dir(&self.protected.paths),
                unmatchable,
            )?,
            tiers,
            scope_paths: policy_matcher("scope.paths", &self.scope.paths, unmatchable)?,
            scope_branches: policy_matcher(
                "scope.branches",
                &self.scope.branches,
                unmatchable_branch,
            )?,
            protected_branches: policy_matcher(
                "fences.protected_branches",
                &self.fences.protected_branches,
                unmatchable_branch,
            )?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse::parse;
    use crate::schema::{Fences, Integrity, Protected, Scope, strings};
    use crate::validate::validate;

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
            ("docs/*.md", "docs/\u{e9}.md", true),
            ("docs/?.md", "docs/\u{e9}.md", false),
            ("docs/??.md", "docs/\u{e9}.md", true),
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
            ("{/src/**,x}", ALTERNATION),
            ("{src/,lib/}", ALTERNATION),
            ("src/{a,b}/**", ALTERNATION),
            ("src/{", ALTERNATION),
            ("}", ALTERNATION),
            ("[/]src/**", CLASS),
            ("[.]/src/**", CLASS),
            ("src/[a]/**", CLASS),
            ("[", CLASS),
            ("src/]", CLASS),
            ("src//**", COMPONENT),
            ("src/./**", COMPONENT),
            ("src/../src/**", COMPONENT),
            ("..", COMPONENT),
            (".", COMPONENT),
            ("a\tb", CHARACTER),
            ("\t", CHARACTER),
            ("\u{200b}", CHARACTER),
            ("\u{feff}.github/**", CHARACTER),
            (".github/**\u{a0}", CHARACTER),
            ("\u{3000}", CHARACTER),
            (".github/**\u{3164}", CHARACTER),
            ("src/auth\u{115f}/**", CHARACTER),
            ("src/\u{430}uth/**", CHARACTER),
            ("\u{ff53}rc/**", CHARACTER),
            ("docs/\u{e9}t\u{e9}/**", CHARACTER),
            ("\u{6587}\u{6863}/**", CHARACTER),
            ("!docs/**", NEGATION),
            (".github\u{ff0f}**", CHARACTER),
            (" src/**", EDGE),
            ("src/** ", EDGE),
            ("", EMPTY),
            ("/src/**", LEADING),
            ("/", LEADING),
            ("./src/**", LEADING),
            ("./", LEADING),
            ("src/", TRAILING),
            ("**/", TRAILING),
            (r"src\**", BACKSLASH),
            (r"a\*b", BACKSLASH),
            (r"\", BACKSLASH),
        ];
        let sections = [
            "[protected]\npaths",
            "[tiers.high]\npaths",
            "[scope]\npaths",
            "[scope]\nbranches",
            "[fences]\nprotected_branches",
        ];
        for section in sections {
            for (pattern, reason) in table {
                let text = format!("version = 1\n{section} = ['ok', '{pattern}']");
                let err = parse(&text).unwrap_err();
                let PolicyError::Glob { key, detail, .. } = &err else {
                    panic!("{text}: {err:?}")
                };
                assert!(key.ends_with("[1]"), "{text}: {key}");
                assert_eq!(detail, reason, "{text}");
            }
        }
    }

    /// TOML literal strings carry no control char but tab, so these bypass `parse`.
    #[test]
    fn control_chars_and_del_are_refused() {
        for pattern in ["a\rb", "a\nb", "\u{1b}m", "a\u{7f}b", "\0"] {
            assert_eq!(unmatchable(pattern), Some(CHARACTER), "{pattern:?}");
        }
    }

    #[test]
    fn branch_names_git_refuses_are_refused() {
        let refused = [
            "HEAD",
            "-",
            "-main",
            "-*",
            "release 1.0",
            "v1..2",
            "main.lock",
            "x.lock/y",
            ".hidden",
            "release/.x",
            "main.",
            "*.lock",
            ".*",
            "a~b",
            "a^b",
            "a:b",
        ];
        let accepted = [
            "main",
            "@",
            "v1.2",
            "feature/a.b",
            "release/*",
            "*",
            "**",
            "a.b/c",
            "a.locked",
            "main.LOCK",
            "a-b",
        ];
        for pattern in refused {
            assert_eq!(unmatchable_branch(pattern), Some(BRANCH), "{pattern}");
        }
        for pattern in accepted {
            assert_eq!(unmatchable_branch(pattern), None, "{pattern}");
        }
        for section in ["[scope]\nbranches", "[fences]\nprotected_branches"] {
            let text = format!("version = 1\n{section} = ['ok', 'main.lock']");
            let err = parse(&text).unwrap_err();
            let PolicyError::Glob { key, detail, .. } = &err else {
                panic!("{text}: {err:?}")
            };
            assert_eq!(
                (key.as_str().ends_with("[1]"), detail.as_str()),
                (true, BRANCH)
            );
        }
        for section in [
            "[protected]\npaths",
            "[tiers.high]\npaths",
            "[scope]\npaths",
        ] {
            let text = format!("version = 1\n{section} = ['.github/**', 'my dir/**', 'x.lock']");
            assert!(parse(&text).is_ok(), "{text}");
        }
    }

    #[test]
    fn printable_ascii_patterns_are_accepted() {
        let table = ["**/*.rs", "release/*", "a~b", "docs/my file.md", "v1..2"];
        for pattern in table {
            assert_eq!(unmatchable(pattern), None, "{pattern}");
            let text = format!("version = 1\n[protected]\npaths = ['{pattern}']");
            assert!(parse(&text).is_ok(), "{pattern}");
        }
    }

    #[test]
    fn matchers_protect_jtac_even_on_a_literal_policy() {
        let policy = Policy {
            version: 1,
            checks: BTreeMap::new(),
            integrity: Integrity::default(),
            protected: Protected { paths: vec![] },
            tiers: BTreeMap::new(),
            fences: Fences::default(),
            scope: Scope::default(),
        };
        let protected = policy.matchers().unwrap().protected;
        assert!(protected.is_match(".jtac/policy.toml"));
        assert_eq!(validate(&policy), Err(PolicyError::JtacUnprotected));
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
