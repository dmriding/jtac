//! One check of a policy, versions 1 and 2 (DESIGN.md sections 10.2 and 15.2). `CheckFile` is
//! the shape on disk and in the hash: a `run` check keeps version 1's keys, so its hash stands.

use jtac_core::Pin;
use serde::{Deserialize, Serialize};

/// One configured check.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "CheckFile", into = "CheckFile")]
pub struct Check {
    /// Who produces the result.
    pub source: Source,
    /// Whether the verdict needs this check to pass.
    pub required: bool,
}

/// Who produces a check's result: one or the other, never both and never neither.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Source {
    /// jtac runs it at every gate.
    Run {
        /// The argv to spawn. No shell.
        argv: Vec<String>,
        /// Seconds before the check is killed.
        timeout_secs: u64,
    },
    /// The team's CI ran it; the merge job reads the result. Policy version 2.
    Ci(Pin),
}

impl Check {
    /// The pin of a check read from CI; `None` for one jtac runs.
    pub fn pin(&self) -> Option<&Pin> {
        match &self.source {
            Source::Ci(pin) => Some(pin),
            Source::Run { .. } => None,
        }
    }
}

/// Seconds a check runs before it is killed, unless the policy says otherwise.
pub const DEFAULT_TIMEOUT_SECS: u64 = 600;

const BOTH: &str = "a check has `run` or `ci`, never both";
const NEITHER: &str = "a check needs `run`, or `ci` under policy version 2";
const TIMEOUT: &str = "`timeout_secs` belongs to `run`: jtac does not run a `ci` check";

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct CheckFile {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    run: Option<Vec<String>>,
    #[serde(default = "yes")]
    required: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    timeout_secs: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    ci: Option<Pin>,
}

fn yes() -> bool {
    true
}

impl TryFrom<CheckFile> for Check {
    type Error = &'static str;

    fn try_from(file: CheckFile) -> Result<Self, Self::Error> {
        let source = match (file.run, file.ci, file.timeout_secs) {
            (Some(argv), None, timeout) => Source::Run {
                argv,
                timeout_secs: timeout.unwrap_or(DEFAULT_TIMEOUT_SECS),
            },
            (None, Some(pin), None) => Source::Ci(pin),
            (None, Some(_), Some(_)) => return Err(TIMEOUT),
            (Some(_), Some(_), _) => return Err(BOTH),
            (None, None, _) => return Err(NEITHER),
        };
        Ok(Self {
            source,
            required: file.required,
        })
    }
}

impl From<Check> for CheckFile {
    fn from(check: Check) -> Self {
        let (run, timeout_secs, ci) = match check.source {
            Source::Run { argv, timeout_secs } => (Some(argv), Some(timeout_secs), None),
            Source::Ci(pin) => (None, None, Some(pin)),
        };
        Self {
            run,
            required: check.required,
            timeout_secs,
            ci,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use jtac_core::canonical_bytes;

    fn read(toml: &str) -> Result<Check, String> {
        toml::from_str(toml).map_err(|e| e.message().to_owned())
    }

    #[test]
    fn a_run_check_writes_the_keys_version_1_hashed() {
        let check = read("run = [\"cargo\", \"test\"]").unwrap();
        assert_eq!(
            String::from_utf8(canonical_bytes(&check).unwrap()).unwrap(),
            r#"{"required":true,"run":["cargo","test"],"timeout_secs":600}"#
        );
    }

    #[test]
    fn a_ci_check_writes_its_pin_and_no_timeout() {
        let table = [
            (
                "ci = { workflow = \".github/workflows/ci.yml\", job = \"test\" }",
                r#"{"ci":{"job":"test","workflow":".github/workflows/ci.yml"},"required":true}"#,
            ),
            (
                "ci = { workflow = \"ci.yml\" }\nrequired = false",
                r#"{"ci":{"workflow":"ci.yml"},"required":false}"#,
            ),
        ];
        for (toml, want) in table {
            let check = read(toml).unwrap();
            assert!(check.pin().is_some(), "{toml}");
            let bytes = canonical_bytes(&check).unwrap();
            assert_eq!(String::from_utf8(bytes).unwrap(), want);
        }
    }

    #[test]
    fn both_neither_a_ci_timeout_and_an_unknown_key_are_refused() {
        let ci = "ci = { workflow = \"ci.yml\" }";
        let table = [
            (format!("run = [\"x\"]\n{ci}"), BOTH),
            (format!("run = [\"x\"]\n{ci}\ntimeout_secs = 5"), BOTH),
            ("required = true".to_owned(), NEITHER),
            ("timeout_secs = 5".to_owned(), NEITHER),
            (format!("{ci}\ntimeout_secs = 5"), TIMEOUT),
            (format!("{ci}\nwhen = \"x\""), "unknown field `when`"),
            (
                "ci = { workflow = \"ci.yml\", jobs = \"x\" }".to_owned(),
                "unknown field `jobs`",
            ),
            (
                "ci = { job = \"x\" }".to_owned(),
                "missing field `workflow`",
            ),
            ("ci = \"ci.yml\"".to_owned(), "invalid type"),
        ];
        for (toml, want) in table {
            let got = read(&toml).unwrap_err();
            assert!(got.contains(want), "{toml}: {got}");
        }
    }
}
