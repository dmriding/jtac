//! What `jtac init` reads from the work tree root to choose a policy (DESIGN.md sections 10.2
//! and 11.7), and the policy text it writes. The mapping from facts to policy is
//! `jtac_policy::default_policy`; this file only gathers the facts.

use std::collections::BTreeSet;
use std::path::Path;

use jtac_core::Sha256Digest;
use jtac_policy::{Detected, Js, POLICY_PATH, PolicyError, Runner, default_policy, policy_hash};
use serde_json::Value;

use crate::init::ask;
use crate::refuse::Refusal;

const PYTHON: &[&str] = &[
    "pyproject.toml",
    "setup.py",
    "setup.cfg",
    "requirements.txt",
];

/// Lockfile to runner, first match wins; none is npm.
const LOCKS: &[(&str, Runner)] = &[
    ("pnpm-lock.yaml", Runner::Pnpm),
    ("yarn.lock", Runner::Yarn),
    ("bun.lockb", Runner::Bun),
    ("bun.lock", Runner::Bun),
];

/// The facts from `file`, which says whether a root-level name is a file, and the text of
/// `package.json` when there is one. A manifest that is not a JSON object with an object of
/// `scripts` is an error, never a package with no scripts.
pub fn detected(file: &dyn Fn(&str) -> bool, package: Option<&str>) -> Result<Detected, String> {
    let javascript = package
        .map(|text| {
            let manifest: Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
            let Value::Object(manifest) = manifest else {
                return Err("it is not a JSON object".to_owned());
            };
            let scripts: BTreeSet<String> = match manifest.get("scripts") {
                None => BTreeSet::new(),
                Some(Value::Object(s)) => s.keys().cloned().collect(),
                Some(_) => return Err("`scripts` is not an object".to_owned()),
            };
            let runner = LOCKS
                .iter()
                .find(|(lock, _)| file(lock))
                .map_or(Runner::Npm, |(_, r)| *r);
            Ok(Js { runner, scripts })
        })
        .transpose()?;
    Ok(Detected {
        rust: file("Cargo.toml"),
        python: PYTHON.iter().any(|n| file(n)),
        go: file("go.mod"),
        javascript,
    })
}

/// Read the facts at `root`.
pub fn detect(root: &Path) -> Result<Detected, Refusal> {
    let file = |name: &str| root.join(name).is_file();
    let package = if file("package.json") {
        let text = std::fs::read_to_string(root.join("package.json"));
        Some(text.map_err(|e| Refusal::init_package_json(&e.to_string()))?)
    } else {
        None
    };
    detected(&file, package.as_deref()).map_err(|e| Refusal::init_package_json(&e))
}

/// The languages found, for the report.
pub fn languages(d: &Detected) -> String {
    let js = d.javascript.as_ref().map(|js| match js.runner {
        Runner::Npm => "javascript (npm)",
        Runner::Pnpm => "javascript (pnpm)",
        Runner::Yarn => "javascript (yarn)",
        Runner::Bun => "javascript (bun)",
    });
    let found: Vec<&str> = [(d.rust, "rust"), (d.python, "python"), (d.go, "go")]
        .into_iter()
        .filter_map(|(on, name)| on.then_some(name))
        .chain(js)
        .collect();
    if found.is_empty() {
        "nothing".to_owned()
    } else {
        found.join(", ")
    }
}

/// The policy init writes when the tree has none.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NewPolicy {
    /// The file's text.
    pub text: String,
    /// Its hash, which the lever node names.
    pub hash: Sha256Digest,
    /// How many checks it holds; none leaves only the integrity rules.
    pub checks: usize,
    /// What detection found, for the report.
    pub languages: String,
}

/// The policy for `d`, its text proven to parse back to the same hash, so the file init
/// writes is exactly the policy it chose.
pub fn policy_text(d: &Detected) -> Result<NewPolicy, Refusal> {
    let policy = default_policy(d);
    let text = jtac_policy::to_toml(&policy);
    let hash = policy_hash(&policy).map_err(Refusal::internal)?;
    let back = jtac_policy::parse(&text).map_err(|e| Refusal::internal(e.to_string()))?;
    if policy_hash(&back).map_err(Refusal::internal)? != hash {
        return Err(Refusal::internal(
            "the default policy does not read back to the same hash",
        ));
    }
    Ok(NewPolicy {
        text,
        hash,
        checks: policy.checks.len(),
        languages: languages(d),
    })
}

/// The hash of the policy file init kept, which its lever node names.
pub fn kept_hash(path: &Path) -> Result<Sha256Digest, Refusal> {
    let bytes = std::fs::read(path).map_err(|e| Refusal::init_io("the policy", path, &e))?;
    let text = String::from_utf8(bytes).map_err(|_| Refusal::init_policy(None))?;
    let policy = jtac_policy::parse(&text).map_err(|e| Refusal::init_policy(Some(&e)))?;
    policy_hash(&policy).map_err(Refusal::internal)
}

impl Refusal {
    /// `package.json` could not be read as a manifest.
    pub fn init_package_json(what: &str) -> Self {
        ask(
            format!("`package.json` could not be read: {what}."),
            "init writes a check for each script the manifest declares; a manifest it cannot \
             read would get a policy with none, a gate that looks set up and checks nothing.",
            "Fix `package.json` (`npm pkg get scripts` shows what npm reads), then run `jtac \
             init` again, or write `.jtac/policy.toml` yourself and run `jtac init`.",
        )
    }

    /// The policy in the work tree does not parse; the lever node needs its hash.
    pub fn init_policy(err: Option<&PolicyError>) -> Self {
        let [what, why, how] = err.map_or_else(
            || {
                [
                    "it is not UTF-8".to_owned(),
                    "A policy that does not parse denies.".to_owned(),
                    "Rewrite it as UTF-8 TOML".to_owned(),
                ]
            },
            PolicyError::three_parts,
        );
        ask(
            format!("{POLICY_PATH} in the work tree: {what}"),
            format!(
                "{why} init records its run under the policy's hash and never rewrites a \
                 policy someone wrote."
            ),
            format!("{how}, then run `jtac init` again."),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn js(runner: Runner, scripts: &[&str]) -> Option<Js> {
        let scripts = scripts.iter().map(|s| (*s).to_owned()).collect();
        Some(Js { runner, scripts })
    }

    #[test]
    fn each_manifest_and_lockfile_is_detected_at_the_root() {
        let pkg = r#"{"scripts":{"test":"t","lint":"l"}}"#;
        let rows: [(&[&str], Option<&str>, Detected); 10] = [
            (&[], None, Detected::default()),
            (
                &["Cargo.toml"],
                None,
                Detected {
                    rust: true,
                    ..Detected::default()
                },
            ),
            (
                &["go.mod"],
                None,
                Detected {
                    go: true,
                    ..Detected::default()
                },
            ),
            (
                &["setup.cfg"],
                None,
                Detected {
                    python: true,
                    ..Detected::default()
                },
            ),
            (
                &["requirements.txt"],
                None,
                Detected {
                    python: true,
                    ..Detected::default()
                },
            ),
            (
                &["package.json"],
                Some(pkg),
                Detected {
                    javascript: js(Runner::Npm, &["lint", "test"]),
                    ..Detected::default()
                },
            ),
            (
                &["package.json", "yarn.lock", "bun.lock"],
                Some(r#"{"name":"example-app"}"#),
                Detected {
                    javascript: js(Runner::Yarn, &[]),
                    ..Detected::default()
                },
            ),
            (
                &["package.json", "bun.lock"],
                Some(pkg),
                Detected {
                    javascript: js(Runner::Bun, &["lint", "test"]),
                    ..Detected::default()
                },
            ),
            (
                &["package.json", "pnpm-lock.yaml", "yarn.lock"],
                Some(pkg),
                Detected {
                    javascript: js(Runner::Pnpm, &["lint", "test"]),
                    ..Detected::default()
                },
            ),
            (
                &["Cargo.toml", "pyproject.toml"],
                None,
                Detected {
                    rust: true,
                    python: true,
                    ..Detected::default()
                },
            ),
        ];
        for (names, package, want) in rows {
            let file = |n: &str| names.contains(&n);
            assert_eq!(detected(&file, package).unwrap(), want, "{names:?}");
        }
    }

    #[test]
    fn a_manifest_that_is_not_an_object_with_object_scripts_is_an_error() {
        let file = |n: &str| n == "package.json";
        for bad in ["{", "[]", r#"{"scripts":["test"]}"#, r#""x""#] {
            assert!(detected(&file, Some(bad)).is_err(), "{bad}");
        }
    }

    #[test]
    fn the_written_text_reads_back_to_the_default_policy_hash() {
        let d = Detected {
            rust: true,
            javascript: js(Runner::Pnpm, &["test"]),
            ..Detected::default()
        };
        let new = policy_text(&d).unwrap();
        assert_eq!(new.hash, policy_hash(&default_policy(&d)).unwrap());
        assert!(new.text.contains("[checks.rust-test]"), "{}", new.text);
        assert_eq!(
            (new.checks, new.languages.as_str()),
            (4, "rust, javascript (pnpm)")
        );
        assert_eq!(languages(&Detected::default()), "nothing");
    }
}
