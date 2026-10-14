//! The human text for an `Output`: banner and plain-language reason first (DESIGN.md section
//! 7.2), then the facts, each blocking reason in three parts, the advisory findings, and the
//! checks.

use std::fmt::Write;

use crate::output::Output;
use crate::reason::{finding_line, shown};

/// Render `o`.
pub fn text(o: &Output) -> String {
    let mut s = String::new();
    let _ = writeln!(s, "{} (preview, nothing written): {}", o.banner, summary(o));
    let _ = writeln!(s, "  commit  {}", o.commit);
    let _ = writeln!(s, "  tree    {}", o.tree);
    let _ = writeln!(s, "  base    {} ({})", o.base.commit, o.base.from);
    let _ = writeln!(s, "  policy  {}", o.policy);
    for note in &o.notes {
        let _ = writeln!(s, "  note    {note}");
    }
    for (i, r) in o.reasons.iter().enumerate() {
        let _ = writeln!(s, "  {}. {}", i + 1, r.what);
        let _ = writeln!(s, "     why  {}", r.why);
        let _ = writeln!(s, "     fix  {}", r.how);
    }
    if !o.advisory.is_empty() {
        let _ = writeln!(s, "  advisory");
        for f in &o.advisory {
            let line = finding_line(&f.rule, f.severity, &f.path, f.line, &f.message);
            let _ = writeln!(s, "     {line}");
        }
    }
    let _ = writeln!(s, "  checks");
    for c in &o.checks {
        let argv = if c.argv.is_empty() {
            "diff-integrity rules".to_owned()
        } else {
            shown(&c.argv)
        };
        let _ = writeln!(s, "     {} {} {}ms  {argv}", c.id, c.outcome, c.duration_ms);
    }
    match (o.added_lines, &o.tiers) {
        (Some(n), Some(tiers)) if tiers.is_empty() => {
            let _ = writeln!(s, "  added   {n} lines; tiers none");
        }
        (Some(n), Some(tiers)) => {
            let _ = writeln!(s, "  added   {n} lines; tiers {}", tiers.join(", "));
        }
        _ => {
            let _ = writeln!(s, "  added   unknown: the rules did not run");
        }
    }
    s
}

fn summary(o: &Output) -> String {
    let total = o.checks.len();
    let passed = o
        .checks
        .iter()
        .filter(|c| c.outcome == jtac_core::Outcome::Pass)
        .count();
    let advisory = match o.advisory.len() {
        0 => String::new(),
        n => format!(", {n} advisory"),
    };
    if o.pass {
        format!("{passed} of {total} checks pass, nothing blocks{advisory}")
    } else {
        format!("{} reasons block{advisory}", o.reasons.len())
    }
}
