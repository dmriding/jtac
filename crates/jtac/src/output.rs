//! The one structure a verdict is shown from: `--json` prints it, `render` writes the text
//! from it (DESIGN.md section 7.1). Versioned with `v`, so a reader can pin what it parses.

use jtac_core::{
    Body, Gate, GitOid, Outcome, Scope, Severity, Sha256Digest, SignedLever, State, Ts,
};
use serde::{Deserialize, Serialize};

use crate::judge::Judged;
use crate::reason::{Explained, explain};
use crate::repo::Base;

/// What `jtac check` prints and `state.json` keeps.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Output {
    /// Output schema version.
    pub v: u32,
    /// The command.
    pub command: String,
    /// Nothing was written.
    pub preview: bool,
    /// `CLEARED HOT` or `ABORT`.
    pub banner: String,
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
    /// The root written or found for this commit; absent in preview and on a block.
    pub root: Option<Sha256Digest>,
    /// Facts that change how to read the verdict, such as a dirty work tree.
    pub notes: Vec<String>,
    /// What blocks, in verdict order.
    pub reasons: Vec<Explained>,
    /// Findings below `block_at`.
    pub advisory: Vec<FindingOut>,
    /// Findings at or above `block_at` a rule waiver covered; absent in an older `state.json`.
    #[serde(default)]
    pub waived: Vec<FindingOut>,
    /// Every check node, `integrity` first.
    pub checks: Vec<CheckOut>,
    /// The waivers the verdict applied; absent in a `state.json` an earlier version wrote.
    #[serde(default)]
    pub waivers: Vec<WaiverOut>,
    /// Lines the change added; absent when the rules did not run.
    pub added_lines: Option<u64>,
    /// Tiers the change is in; absent when the rules did not run.
    pub tiers: Option<Vec<String>>,
}

/// A finding as shown.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
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

/// An applied waiver as shown (DESIGN.md section 11.6).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WaiverOut {
    /// The lever node id.
    pub lever: Sha256Digest,
    /// What it covers.
    pub scope: Scope,
    /// Who signed it.
    pub principal: String,
    /// Verbatim.
    pub reason: String,
    /// When it stops applying; absent for a commit waiver.
    pub expires: Option<Ts>,
    /// The agent-session marker it was written under, when one was set.
    pub agent: Option<String>,
}

impl WaiverOut {
    /// From a root's or the store's lever; `None` for a lever that is not a scoped waiver.
    pub fn of(l: &SignedLever) -> Option<Self> {
        let Body::Lever(b) = &l.node.body else {
            return None;
        };
        Some(Self {
            lever: l.node.id().ok()?,
            scope: b.scope.clone()?,
            principal: b.signer.principal.clone(),
            reason: b.reason.clone(),
            expires: b.expires,
            agent: b.agent.clone(),
        })
    }

    /// One line: scope, signer, marker, reason, every word through [`plain`].
    pub fn line(&self) -> String {
        let scope = match &self.scope {
            Scope::Commit { commit, gate: None } => format!("commit {}", commit.hex()),
            Scope::Commit {
                commit,
                gate: Some(Gate::Ci),
            } => format!("ci gate on commit {}", commit.hex()),
            Scope::Rule { rule, path } => format!("{} under {}", plain(rule), plain(path)),
        };
        let agent = self.agent.as_deref().map_or(String::new(), |m| {
            format!(" under agent session {}", plain(m))
        });
        format!(
            "{scope} by {}{agent}: {}",
            plain(&self.principal),
            plain(&self.reason)
        )
    }
}

/// A stored word with every control character a space: a hand-written node cannot erase the
/// line that flags it or the one above (`a_waiver_line_prints_no_control_character_from_any_field`).
pub fn plain(s: &str) -> String {
    s.chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect()
}

/// A check as shown.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
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
    /// The node this result was cached from, when it was.
    pub cached_from: Option<Sha256Digest>,
}

/// Build the structure from `p`, the caller's `notes`, and the root it wrote or found.
pub fn output(p: &Judged, notes: Vec<String>, preview: bool, root: Option<Sha256Digest>) -> Output {
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
    let mut waived = Vec::new();
    let mut checks = Vec::new();
    for n in &p.nodes {
        match &n.body {
            Body::Finding(f) if !blocking(f) => {
                let shown = FindingOut {
                    rule: f.rule.clone(),
                    severity: f.severity,
                    path: f.path.clone(),
                    line: f.line,
                    message: f.message.clone(),
                };
                if f.severity <= p.facts.block_at {
                    waived.push(shown);
                } else {
                    advisory.push(shown);
                }
            }
            Body::Check(c) => checks.push(CheckOut {
                id: c.id.clone(),
                outcome: c.outcome,
                exit: c.exit,
                duration_ms: c.duration_ms,
                tool_version: c.tool_version.clone(),
                argv: c.argv.clone(),
                cached_from: c.cached_from,
            }),
            _ => {}
        }
    }
    let waivers = p.levers.iter().filter_map(WaiverOut::of).collect();
    Output {
        v: 1,
        command: "check".to_owned(),
        preview,
        banner: if p.verdict.pass {
            "CLEARED HOT"
        } else {
            "ABORT"
        }
        .to_owned(),
        pass: p.verdict.pass,
        state: p.verdict.state,
        ts: p.now,
        commit: p.head.commit,
        tree: p.head.tree,
        base: p.base.clone(),
        policy: p.facts.policy,
        root,
        notes,
        reasons,
        advisory,
        waived,
        checks,
        waivers,
        added_lines: p.report.as_ref().map(|r| r.added_lines),
        tiers: p.report.as_ref().map(|r| r.tiers.iter().cloned().collect()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_waiver_line_prints_no_control_character_from_any_field() {
        let w = WaiverOut {
            lever: Sha256Digest::of(b""),
            scope: Scope::Rule {
                rule: "JT-X\r".into(),
                path: "src\x1b[2K".into(),
            },
            principal: "a\nb".into(),
            reason: "r\tr".into(),
            expires: None,
            agent: Some("M\x7f".into()),
        };
        let line = w.line();
        assert!(!line.chars().any(char::is_control), "{line:?}");
        assert_eq!(
            line,
            "JT-X  under src [2K by a b under agent session M : r r"
        );
    }
}
