//! `jtac audit` (DESIGN.md sections 7.1 and 11.9): one row per commit of the range with its
//! coverage state and the root behind it, then the totals. A report: exit 0 whatever it says.

use std::fmt::Write;
use std::path::Path;

use jtac_gate::Trust;
use serde::Serialize;

use crate::Printed;
use crate::cli::RangeArgs;
use crate::refuse::Refusal;
use crate::survey::{RowOut, survey, via_text};

/// What `jtac audit` prints.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct AuditOut {
    /// Output schema version.
    pub v: u32,
    /// The command.
    pub command: &'static str,
    /// The range as listed.
    pub range: Vec<String>,
    /// Where trust came from.
    pub signers: String,
    /// One per commit, newest first.
    pub rows: Vec<RowOut>,
    /// The totals.
    pub totals: Totals,
    /// Skipped refs, parent roots this clone lacks.
    pub notes: Vec<String>,
}

/// Commits by state, and what the roots said.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Totals {
    /// Commits in the range.
    pub commits: u64,
    /// Covered, by any route.
    pub covered: u64,
    /// Declared off, by any route.
    pub declared_off: u64,
    /// Neither.
    pub absent: u64,
    /// Of the covered and declared off, those by a tree match.
    pub by_tree: u64,
    /// Roots that bear on the range and do not count.
    pub invalid_roots: u64,
    /// Roots whose signer no trust file names.
    pub unknown_signers: u64,
    /// `covered` plus `declared_off` as a whole percentage of `commits`, rounded down.
    pub percent_covered: u64,
}

/// Gather and render.
pub fn audit(a: &RangeArgs, cwd: &Path) -> Result<Printed, Refusal> {
    let s = survey(cwd, "audit", &a.range, a.signers.as_deref())?;
    let (covered, declared_off, absent) = s.totals();
    let commits = s.rows.len() as u64;
    let rows = s.rows_out();
    let out = AuditOut {
        v: 1,
        command: "audit",
        range: s.range.clone(),
        signers: s.signers.describe(),
        totals: Totals {
            commits,
            covered,
            declared_off,
            absent,
            by_tree: rows.iter().filter(|r| r.via == Some("tree")).count() as u64,
            invalid_roots: s
                .ledger
                .roots
                .iter()
                .filter(|r| r.invalid.is_some())
                .count() as u64,
            unknown_signers: s
                .ledger
                .roots
                .iter()
                .filter(|r| r.invalid.is_none() && !matches!(r.trust, Trust::Trusted))
                .count() as u64,
            percent_covered: ((covered + declared_off) * 100)
                .checked_div(commits)
                .unwrap_or_default(),
        },
        rows,
        notes: s.ledger.notes,
    };
    let text = if a.json {
        crate::render::json(&out)?
    } else {
        render(&out)
    };
    Ok(Printed::verdict(text, true))
}

fn render(o: &AuditOut) -> String {
    let mut s = String::new();
    let t = &o.totals;
    let _ = writeln!(
        s,
        "AUDIT {}: {} commits, {} covered, {} declared off, {} absent ({}% covered); {} roots \
         invalid, {} signed by unknown keys",
        o.range.join(" "),
        t.commits,
        t.covered,
        t.declared_off,
        t.absent,
        t.percent_covered,
        t.invalid_roots,
        t.unknown_signers
    );
    let _ = writeln!(s, "  signers {}", o.signers);
    for r in &o.rows {
        let who = match (&r.signer, r.trust) {
            (Some(signer), Some(trust)) => format!(
                "  {} {trust}  policy {}  gate {}",
                signer.principal,
                r.policy_version.unwrap_or_default(),
                r.gate_version.clone().unwrap_or_default()
            ),
            _ => String::new(),
        };
        let _ = writeln!(
            s,
            "  {} {:<12} {}{who}",
            r.commit.hex(),
            r.state,
            via_text(r)
        );
    }
    for note in &o.notes {
        let _ = writeln!(s, "  note    {note}");
    }
    s
}
