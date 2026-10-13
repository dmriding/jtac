//! Detected facts to the policy `init` writes (DESIGN.md sections 10.2 and 11.7). Detection
//! reads the tree and is the CLI's; this mapping is pure. Ids always carry a language prefix,
//! so detecting a second language never renames a check.

use std::collections::{BTreeMap, BTreeSet};

use crate::schema::{
    Check, DEFAULT_TIMEOUT_SECS, Fences, Integrity, Policy, Protected, Scope, strings,
};

/// What the CLI found in the tree.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Detected {
    /// A Cargo manifest.
    pub rust: bool,
    /// A Python project.
    pub python: bool,
    /// A Go module.
    pub go: bool,
    /// A JavaScript or TypeScript package.
    pub javascript: Option<Js>,
}

/// A JavaScript or TypeScript package.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Js {
    /// The package manager its lockfile names.
    pub runner: Runner,
    /// Script names its manifest declares.
    pub scripts: BTreeSet<String>,
}

/// A JavaScript package manager.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Runner {
    /// `npm`.
    Npm,
    /// `pnpm`.
    Pnpm,
    /// `yarn`.
    Yarn,
    /// `bun`.
    Bun,
}

impl Runner {
    fn program(self) -> &'static str {
        match self {
            Self::Npm => "npm",
            Self::Pnpm => "pnpm",
            Self::Yarn => "yarn",
            Self::Bun => "bun",
        }
    }
}

type Row = (&'static str, &'static [&'static str], u64);

const RUST: &[Row] = &[
    (
        "fmt",
        &["cargo", "fmt", "--all", "--", "--check"],
        DEFAULT_TIMEOUT_SECS,
    ),
    (
        "lint",
        &[
            "cargo",
            "clippy",
            "--workspace",
            "--all-targets",
            "--",
            "-D",
            "warnings",
        ],
        DEFAULT_TIMEOUT_SECS,
    ),
    ("test", &["cargo", "test", "--workspace"], 1200),
];

const PYTHON: &[Row] = &[
    (
        "fmt",
        &["ruff", "format", "--check", "."],
        DEFAULT_TIMEOUT_SECS,
    ),
    ("lint", &["ruff", "check", "."], DEFAULT_TIMEOUT_SECS),
    ("test", &["python", "-m", "pytest"], DEFAULT_TIMEOUT_SECS),
];

const GO: &[Row] = &[
    ("lint", &["go", "vet", "./..."], DEFAULT_TIMEOUT_SECS),
    ("test", &["go", "test", "./..."], DEFAULT_TIMEOUT_SECS),
];

/// JS check kind to the scripts that fill it, first match wins. `format` and `fmt` rewrite
/// files and exit 0, so they are never a format check.
const JS_SCRIPTS: &[(&str, &[&str])] = &[
    ("fmt", &["format:check", "fmt:check", "prettier:check"]),
    ("lint", &["lint"]),
    ("test", &["test"]),
];

/// The policy `init` writes for `d`. Nothing detected is a valid policy with no checks.
pub fn default_policy(d: &Detected) -> Policy {
    let mut checks = BTreeMap::new();
    let mut add = |prefix: &str, kind: &str, run: Vec<String>, timeout_secs| {
        let check = Check {
            run,
            required: true,
            timeout_secs,
        };
        checks.insert(format!("{prefix}-{kind}"), check);
    };
    for (found, prefix, rows) in [
        (d.rust, "rust", RUST),
        (d.python, "py", PYTHON),
        (d.go, "go", GO),
    ] {
        for (kind, run, timeout) in rows.iter().filter(|_| found) {
            add(prefix, kind, strings(run), *timeout);
        }
    }
    if let Some(js) = &d.javascript {
        for (kind, names) in JS_SCRIPTS {
            if let Some(script) = names.iter().find(|s| js.scripts.contains(**s)) {
                add(
                    "js",
                    kind,
                    strings(&[js.runner.program(), "run", script]),
                    DEFAULT_TIMEOUT_SECS,
                );
            }
        }
    }
    Policy {
        version: 1,
        checks,
        integrity: Integrity::default(),
        protected: Protected::default(),
        tiers: BTreeMap::new(),
        fences: Fences::default(),
        scope: Scope::default(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hash::tests::design_example;
    use crate::{parse, policy_hash, to_toml, validate};

    fn checks(rows: &[Row]) -> BTreeMap<String, Check> {
        rows.iter()
            .map(|(id, run, timeout_secs)| {
                let check = Check {
                    run: strings(run),
                    required: true,
                    timeout_secs: *timeout_secs,
                };
                ((*id).to_owned(), check)
            })
            .collect()
    }

    fn js(runner: Runner, scripts: &[&str]) -> Detected {
        let scripts = scripts.iter().map(|s| (*s).to_owned()).collect();
        Detected {
            javascript: Some(Js { runner, scripts }),
            ..Detected::default()
        }
    }

    #[test]
    fn rust_defaults_are_the_design_example_checks() {
        let policy = default_policy(&Detected {
            rust: true,
            ..Detected::default()
        });
        let want = checks(&[
            ("rust-fmt", &["cargo", "fmt", "--all", "--", "--check"], 600),
            (
                "rust-lint",
                &[
                    "cargo",
                    "clippy",
                    "--workspace",
                    "--all-targets",
                    "--",
                    "-D",
                    "warnings",
                ],
                600,
            ),
            ("rust-test", &["cargo", "test", "--workspace"], 1200),
        ]);
        assert_eq!(policy.checks, want);
        let mut design = parse(design_example()).unwrap();
        design.checks = (design.checks.into_iter())
            .map(|(id, check)| (format!("rust-{id}"), check))
            .collect();
        design.tiers.clear();
        assert_eq!(policy, design);
    }

    #[test]
    fn python_defaults_are_ruff_and_pytest() {
        let policy = default_policy(&Detected {
            python: true,
            ..Detected::default()
        });
        let want = checks(&[
            ("py-fmt", &["ruff", "format", "--check", "."], 600),
            ("py-lint", &["ruff", "check", "."], 600),
            ("py-test", &["python", "-m", "pytest"], 600),
        ]);
        assert_eq!(policy.checks, want);
    }

    #[test]
    fn go_defaults_are_vet_and_test_with_no_fmt() {
        let policy = default_policy(&Detected {
            go: true,
            ..Detected::default()
        });
        let want = checks(&[
            ("go-lint", &["go", "vet", "./..."], 600),
            ("go-test", &["go", "test", "./..."], 600),
        ]);
        assert_eq!(policy.checks, want);
    }

    #[test]
    fn js_defaults_run_only_the_scripts_the_manifest_declares() {
        let table: &[(Runner, &[&str], &[Row])] = &[
            (Runner::Npm, &["format", "fmt", "prettier"], &[]),
            (Runner::Npm, &[], &[]),
            (Runner::Npm, &["build", "dev"], &[]),
            (
                Runner::Pnpm,
                &["test"],
                &[("js-test", &["pnpm", "run", "test"], 600)],
            ),
            (
                Runner::Yarn,
                &["prettier:check", "lint"],
                &[
                    ("js-fmt", &["yarn", "run", "prettier:check"], 600),
                    ("js-lint", &["yarn", "run", "lint"], 600),
                ],
            ),
            (
                Runner::Npm,
                &["prettier:check", "fmt:check"],
                &[("js-fmt", &["npm", "run", "fmt:check"], 600)],
            ),
            (
                Runner::Bun,
                &[
                    "format",
                    "prettier:check",
                    "fmt:check",
                    "format:check",
                    "test",
                ],
                &[
                    ("js-fmt", &["bun", "run", "format:check"], 600),
                    ("js-test", &["bun", "run", "test"], 600),
                ],
            ),
        ];
        for (runner, scripts, want) in table {
            assert_eq!(
                default_policy(&js(*runner, scripts)).checks,
                checks(want),
                "{scripts:?}"
            );
        }
    }

    #[test]
    fn nothing_detected_is_a_valid_policy_with_no_checks() {
        let policy = default_policy(&Detected::default());
        assert!(policy.checks.is_empty());
        assert_eq!(validate(&policy), Ok(()));
    }

    #[test]
    fn two_languages_keep_their_prefixed_ids() {
        let both = Detected {
            rust: true,
            python: true,
            ..Detected::default()
        };
        let ids: Vec<_> = default_policy(&both).checks.into_keys().collect();
        assert_eq!(
            ids,
            [
                "py-fmt",
                "py-lint",
                "py-test",
                "rust-fmt",
                "rust-lint",
                "rust-test"
            ]
        );
    }

    #[test]
    fn every_default_validates_hashes_and_round_trips() {
        let all = ["format:check", "lint", "test"];
        for bits in 0..16u8 {
            let mut d = js(Runner::Npm, &all);
            if bits & 8 == 0 {
                d.javascript = None;
            }
            d.rust = bits & 1 != 0;
            d.python = bits & 2 != 0;
            d.go = bits & 4 != 0;
            let policy = default_policy(&d);
            assert_eq!(validate(&policy), Ok(()), "{d:?}");
            let again = parse(&to_toml(&policy)).unwrap();
            assert_eq!(
                policy_hash(&again).unwrap(),
                policy_hash(&policy).unwrap(),
                "{d:?}"
            );
        }
    }
}
