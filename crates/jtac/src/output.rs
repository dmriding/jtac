//! The one structure a verdict is shown from: `--json` prints it, `render` writes the text
//! from it (DESIGN.md section 7.1). Versioned with `v`, so a reader can pin what it parses.

use jtac_core::{Body, GitOid, Outcome, Severity, Sha256Digest, State, Ts};
use serde::Serialize;

use crate::preview::Preview;
use crate::reason::{Explained, explain};
use crate::repo::Base;

/// What `jtac check --preview` prints.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Output {
    /// Output schema version.
    pub v: u32,
    /// The command.
    pub command: &'static str,
    /// Nothing was written.
    pub preview: bool,
    /// `CLEARED HOT` or `ABORT`.
    pub banner: &'static str,
    /// Whether a root could be written.
    pub pass: bool,
    /// Covered or declared off.
    pub state: State,
    /// The instant every node carries.
    pub ts: Ts,
    /// The commit under test.
    pub commit: GitOid,
    /// Its tree.
    pub tree: GitOid,
    /// What the rules diffed from.
    pub base: Base,
    /// Hash of the resolved policy.
    pub policy: Sha256Digest,
    /// Facts that change how to read the verdict, such as a dirty work tree.
    pub notes: Vec<String>,
    /// What blocks, in verdict order.
    pub reasons: Vec<Explained>,
    /// Findings below `block_at`.
    pub advisory: Vec<FindingOut>,
    /// Every check node, `integrity` first.
    pub checks: Vec<CheckOut>,
    /// Lines the change added; absent when the rules did not run.
    pub added_lines: Option<u64>,
    /// Tiers the change is in; absent when the rules did not run.
    pub tiers: Option<Vec<String>>,
}

/// A finding as shown.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct FindingOut {
    /// Rule id.
    pub rule: String,
    /// Severity.
    pub severity: Severity,
    /// Path.
    pub path: String,
    /// Line, zero when none.
    pub line: u64,
    /// Detail.
    pub message: String,
}

/// A check as shown.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct CheckOut {
    /// Check id.
    pub id: String,
    /// Outcome.
    pub outcome: Outcome,
    /// Exit code.
    pub exit: i32,
    /// Wall time.
    pub duration_ms: u64,
    /// What the version probe printed.
    pub tool_version: String,
    /// The argv, empty for `integrity`.
    pub argv: Vec<String>,
}

/// Build the structure from `p` and the caller's `notes`.
pub fn output(p: &Preview, notes: Vec<String>) -> Output {
    let reasons: Vec<Explained> = p.verdict.reasons.iter().map(|r| explain(r, p)).collect();
    let blocking = |f: &jtac_core::FindingBody| {
        p.verdict.reasons.iter().any(|r| match r {
            jtac_core::Reason::Finding {
                rule,
                severity,
                path,
                line,
                message,
            } => {
                *rule == f.rule
                    && *severity == f.severity
                    && *path == f.path
                    && *line == f.line
                    && *message == f.message
            }
            _ => false,
        })
    };
    let mut advisory = Vec::new();
    let mut checks = Vec::new();
    for n in &p.nodes {
        match &n.body {
            Body::Finding(f) if !blocking(f) => advisory.push(FindingOut {
                rule: f.rule.clone(),
                severity: f.severity,
                path: f.path.clone(),
                line: f.line,
                message: f.message.clone(),
            }),
            Body::Check(c) => checks.push(CheckOut {
                id: c.id.clone(),
                outcome: c.outcome,
                exit: c.exit,
                duration_ms: c.duration_ms,
                tool_version: c.tool_version.clone(),
                argv: c.argv.clone(),
            }),
            _ => {}
        }
    }
    Output {
        v: 1,
        command: "check",
        preview: true,
        banner: if p.verdict.pass {
            "CLEARED HOT"
        } else {
            "ABORT"
        },
        pass: p.verdict.pass,
        state: p.verdict.state,
        ts: p.now,
        commit: p.head.commit,
        tree: p.head.tree,
        base: p.base.clone(),
        policy: p.facts.policy,
        notes,
        reasons,
        advisory,
        checks,
        added_lines: p.report.as_ref().map(|r| r.added_lines),
        tiers: p.report.as_ref().map(|r| r.tiers.iter().cloned().collect()),
    }
}
