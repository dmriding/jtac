//! The policy file's shape, version 1 (DESIGN.md section 10.2). Every section is optional and
//! defaulted; the serialized form is the resolved policy that gets hashed (decision 25).

use std::collections::BTreeMap;

use jtac_core::Severity;
use serde::{Deserialize, Serialize};

/// A resolved policy: the file as written with every default filled in.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Policy {
    /// Schema version. Only 1 is read.
    pub version: u32,
    /// Checks by id. Every required check runs every time.
    #[serde(default)]
    pub checks: BTreeMap<String, Check>,
    /// Diff-integrity settings (DESIGN.md section 10.3).
    #[serde(default)]
    pub integrity: Integrity,
    /// Paths whose change escalates.
    #[serde(default)]
    pub protected: Protected,
    /// Path sets by tier name.
    #[serde(default)]
    pub tiers: BTreeMap<String, Tier>,
    /// Git operations the gate refuses.
    #[serde(default)]
    pub fences: Fences,
    /// Where an agent is cleared to work. Never narrows what the gate checks.
    #[serde(default)]
    pub scope: Scope,
}

/// One configured check.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Check {
    /// The argv to spawn. No shell.
    pub run: Vec<String>,
    /// Whether the verdict needs this check to pass.
    #[serde(default = "yes")]
    pub required: bool,
    /// Seconds before the check is killed.
    #[serde(default = "default_timeout")]
    pub timeout_secs: u64,
}

/// Seconds a check runs before it is killed, unless the policy says otherwise.
pub const DEFAULT_TIMEOUT_SECS: u64 = 600;

fn yes() -> bool {
    true
}

fn default_timeout() -> u64 {
    DEFAULT_TIMEOUT_SECS
}

/// Diff-integrity thresholds.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Integrity {
    /// Lowest finding severity that fails the verdict. Default major; minor is advisory.
    /// `Severity` orders `Blocker` first, so "at or above `block_at`" is `severity <= block_at`.
    pub block_at: Severity,
    /// New suppressions allowed before `JT-SUP-001` blocks.
    pub max_new_suppressions: u64,
    /// Whether removing a test is allowed.
    pub allow_removed_tests: bool,
    /// Whether newly skipping a test is allowed.
    pub allow_newly_skipped_tests: bool,
    /// Production-line cap per file.
    pub max_file_lines: u64,
    /// Line cap per function.
    pub max_function_lines: u64,
    /// Added-line cap per diff. Absent means none.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_added_lines: Option<u64>,
}

impl Default for Integrity {
    fn default() -> Self {
        Self {
            block_at: Severity::Major,
            max_new_suppressions: 0,
            allow_removed_tests: false,
            allow_newly_skipped_tests: false,
            max_file_lines: 800,
            max_function_lines: 150,
            max_added_lines: None,
        }
    }
}

/// Protected paths. `.jtac/**` is always among them (DESIGN.md section 10.1).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Protected {
    /// Globs over repository-relative paths.
    pub paths: Vec<String>,
}

/// The pattern that keeps the policy itself protected.
pub const JTAC_DIR_GLOB: &str = ".jtac/**";

/// `paths` with `.jtac/**` appended when absent. Appended, so `paths[i]` in an error still
/// points at the entry written. `parse` and `Policy::matchers` both go through here.
pub(crate) fn with_jtac_dir(paths: &[String]) -> Vec<String> {
    let mut paths = paths.to_vec();
    if !paths.iter().any(|path| path == JTAC_DIR_GLOB) {
        paths.push(JTAC_DIR_GLOB.to_owned());
    }
    paths
}

impl Default for Protected {
    fn default() -> Self {
        Self {
            paths: strings(&[
                JTAC_DIR_GLOB,
                ".github/**",
                "clippy.toml",
                "deny.toml",
                "rust-toolchain.toml",
            ]),
        }
    }
}

/// Paths that put a change in a tier.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Tier {
    /// Globs over repository-relative paths.
    pub paths: Vec<String>,
}

/// Git operations the gate refuses.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Fences {
    /// Branch globs the fences guard.
    pub protected_branches: Vec<String>,
    /// A force push to a protected branch.
    pub force_push_protected: FenceMode,
    /// A commit or push that skips hooks.
    pub no_verify: FenceMode,
}

impl Default for Fences {
    fn default() -> Self {
        Self {
            protected_branches: strings(&["main", "release/*"]),
            force_push_protected: FenceMode::Deny,
            no_verify: FenceMode::Deny,
        }
    }
}

/// Whether a fence holds.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FenceMode {
    /// Refused. The only value `init` writes.
    #[default]
    Deny,
    /// Let through.
    Allow,
}

/// The area an agent is cleared to work in, shown by `jtac scope` and named by the tool-call
/// advisory (DESIGN.md 7.2, 10.2). It never narrows what the gate checks: the gate judges the
/// whole tree.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Scope {
    /// Branch globs.
    pub branches: Vec<String>,
    /// Path globs.
    pub paths: Vec<String>,
}

impl Default for Scope {
    fn default() -> Self {
        Self {
            branches: strings(&["*"]),
            paths: strings(&["**"]),
        }
    }
}

pub(crate) fn strings(items: &[&str]) -> Vec<String> {
    items.iter().map(|s| (*s).to_owned()).collect()
}
