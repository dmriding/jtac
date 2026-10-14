//! What `jtac init` reports: one line per step, then the notes, the allowed-signers line, and
//! the policy it wrote. `--json` prints this structure; the text is rendered from it.

use std::fmt::Write;

use serde::Serialize;

/// What a step did.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub enum Did {
    /// Written this run.
    #[serde(rename = "wrote")]
    Wrote,
    /// Already in place; nothing changed.
    #[serde(rename = "kept")]
    Kept,
    /// A dry run's write.
    #[serde(rename = "would write")]
    WouldWrite,
}

impl Did {
    fn word(self) -> &'static str {
        match self {
            Self::Wrote => "wrote",
            Self::Kept => "kept",
            Self::WouldWrite => "would write",
        }
    }
}

/// One step of DESIGN.md section 11.7.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Step {
    /// `key`, `store`, `policy`, `hook`, `refspec`, or `lever`.
    pub step: &'static str,
    /// What it did.
    pub action: Did,
    /// What and where, starting with the noun.
    pub detail: String,
}

/// The whole report.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct InitOutput {
    /// Shape version.
    pub v: u32,
    /// Always `init`.
    pub command: &'static str,
    /// `--dry-run` was given.
    pub dry_run: bool,
    /// The steps, in order.
    pub steps: Vec<Step>,
    /// What the reader should know or do next.
    pub notes: Vec<String>,
    /// `<principal> <public key>` for `.jtac/signers`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub signers: Option<String>,
    /// The policy text written, or that would be.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub policy: Option<String>,
}

impl InitOutput {
    /// An empty report.
    pub fn new(dry_run: bool) -> Self {
        Self {
            v: 1,
            command: "init",
            dry_run,
            steps: Vec::new(),
            notes: Vec::new(),
            signers: None,
            policy: None,
        }
    }

    /// Record a write, or on a dry run the write that would happen.
    pub fn wrote(&mut self, step: &'static str, detail: String) {
        let action = if self.dry_run {
            Did::WouldWrite
        } else {
            Did::Wrote
        };
        self.steps.push(Step {
            step,
            action,
            detail,
        });
    }

    /// Record a step with nothing to do.
    pub fn kept(&mut self, step: &'static str, detail: String) {
        self.steps.push(Step {
            step,
            action: Did::Kept,
            detail,
        });
    }

    /// The step names that wrote, or would.
    pub fn written(&self) -> Vec<&'static str> {
        (self.steps.iter())
            .filter(|s| s.action != Did::Kept)
            .map(|s| s.step)
            .collect()
    }

    /// The text: a summary first, then a line per step, the notes, the signer, the policy.
    pub fn text(&self) -> String {
        let written = self.written().len();
        let kept = self.steps.len() - written;
        let mut s = match (self.dry_run, written) {
            (true, _) => format!(
                "jtac init (dry run, nothing written): {written} would be written, {kept} kept\n"
            ),
            (false, 0) => format!("jtac init: nothing to do, {kept} kept\n"),
            (false, _) => format!("jtac init: {written} written, {kept} kept\n"),
        };
        for step in &self.steps {
            let _ = writeln!(s, "  {} {}", step.action.word(), step.detail);
        }
        if let Some(line) = &self.signers {
            let _ = writeln!(s, "  allowed signer: {line}");
        }
        for note in &self.notes {
            let _ = writeln!(s, "  note: {note}");
        }
        if let Some(policy) = &self.policy {
            let _ = writeln!(s, "  policy:");
            for line in policy.lines() {
                let indent = if line.is_empty() { "" } else { "    " };
                let _ = writeln!(s, "{indent}{line}");
            }
        }
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_summary_counts_writes_and_a_dry_run_says_nothing_was_written() {
        let mut o = InitOutput::new(false);
        o.wrote("store", "store /workspace/.git/jtac".into());
        o.kept("hook", "hook /workspace/.git/hooks/pre-push".into());
        o.notes.push("a note".into());
        o.signers = Some("dev@example.com ssh-ed25519 AAAA".into());
        o.policy = Some("version = 1\n\n[tiers]\n".into());
        assert_eq!(
            o.text(),
            "jtac init: 1 written, 1 kept\n  wrote store /workspace/.git/jtac\n  kept hook \
             /workspace/.git/hooks/pre-push\n  allowed signer: dev@example.com ssh-ed25519 \
             AAAA\n  note: a note\n  policy:\n    version = 1\n\n    [tiers]\n"
        );
        let mut dry = InitOutput::new(true);
        dry.wrote("store", "store x".into());
        assert!(
            dry.text()
                .starts_with("jtac init (dry run, nothing written): 1 would")
        );
        assert!(dry.text().contains("  would write store x\n"));
        assert_eq!(dry.written(), ["store"]);
        let mut noop = InitOutput::new(false);
        noop.kept("store", "store x".into());
        assert!(
            noop.text()
                .starts_with("jtac init: nothing to do, 1 kept\n")
        );
    }
}
