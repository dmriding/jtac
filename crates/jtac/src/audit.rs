//! `jtac audit` (DESIGN.md sections 7.1 and 11.9): one row per commit of the range with its
//! coverage state and the root behind it, then the totals. A report: exit 0 whatever it says.

use std::fmt::Write;
use std::path::Path;

use serde::Serialize;

use crate::Printed;
use crate::cli::RangeArgs;
use crate::refuse::Refusal;
use crate::survey::{RowOut, WaiverRow, short, survey, via_text};

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
    /// Every waiver a valid root applies.
    pub waivers: Vec<WaiverRow>,
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
    /// The local gate passed and left a `ci` check unread: not covered (section 15.2).
    pub partial: u64,
    /// A policy the reading turns on is not in this clone: not covered (decision 60).
    pub policy_not_held: u64,
    /// No valid root bears.
    pub absent: u64,
    /// Of the covered and declared off, those by a tree match.
    pub by_tree: u64,
    /// Roots that bear on the range and do not count.
    pub invalid_roots: u64,
    /// Roots whose signer, or a waiver's signer, no trust file names.
    pub unknown_signers: u64,
    /// Waivers the valid roots apply.
    pub waivers: u64,
    /// Those written under an agent session.
    pub agent_waivers: u64,
    /// `covered` plus `declared_off` as a whole percentage of `commits`, rounded down.
    pub percent_covered: u64,
}

/// Gather and render.
pub fn audit(a: &RangeArgs, cwd: &Path) -> Result<Printed, Refusal> {
    let s = survey(cwd, "audit", &a.range, a.signers.as_deref(), None)?;
    let t = s.totals();
    let (covered, declared_off) = (t.covered, t.declared_off);
    let commits = s.rows.len() as u64;
    let mut notes = s.ledger.notes.clone();
    if t.not_held > 0 {
        notes.push(format!(
            "{} commits read `policy not held` and are counted not covered: this partial clone \
             lacks a policy file their reading turns on; `jtac verify` names the fetch",
            t.not_held
        ));
    }
    let rows = s.rows_out();
    let waivers = s.waivers_out();
    let out = AuditOut {
        v: 1,
        command: "audit",
        range: s.range.clone(),
        signers: s.signers.describe(),
        totals: Totals {
            commits,
            covered,
            declared_off,
            partial: t.partial,
            policy_not_held: t.not_held,
            absent: t.absent,
            by_tree: rows.iter().filter(|r| r.via == Some("tree")).count() as u64,
            invalid_roots: s
                .ledger
                .roots
                .iter()
                .filter(|r| r.invalid.is_some())
                .count() as u64,
            unknown_signers: s.unknown_signers(),
            waivers: waivers.len() as u64,
            agent_waivers: waivers.iter().filter(|w| w.waiver.agent.is_some()).count() as u64,
            percent_covered: ((covered + declared_off) * 100)
                .checked_div(commits)
                .unwrap_or_default(),
        },
        rows,
        waivers,
        notes,
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
        "AUDIT {}: {} commits, {} covered, {} declared off, {}{} absent ({}% covered); {} roots \
         invalid, {} signed by unknown keys",
        o.range.join(" "),
        t.commits,
        t.covered,
        t.declared_off,
        match t.partial {
            0 => String::new(),
            n => format!("{n} covered with a ci check unread, "),
        },
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
            match r.state {
                "partial" => "covered, ci unread",
                "policy_not_held" => "policy not held",
                state => state,
            },
            via_text(r)
        );
        if let Some(h) = &r.head {
            let _ = writeln!(s, "          {}", h.line());
        }
        if let Some(line) = ci_line(r) {
            let _ = writeln!(s, "          {line}");
        }
        if let Some(line) = record_line(r) {
            let _ = writeln!(s, "          {line}");
        }
    }
    for w in &o.waivers {
        let _ = writeln!(s, "  waiver  {}", w.line());
    }
    for note in &o.notes {
        let _ = writeln!(s, "  note    {note}");
    }
    s
}

/// The failure record that reaches the commit (DESIGN.md section 15.2): the `failed`
/// reading with what failed, or, beside a reading that outranks it, the note that an
/// earlier verdict failed.
fn record_line(r: &RowOut) -> Option<String> {
    let record = r.ci_record.as_ref()?;
    let whose = match record.commit == r.commit {
        true => "its own failure record".to_owned(),
        false => format!("{}'s failure record reaches it", short(&record.commit)),
    };
    let lead = match r.ci {
        Some("failed") => "ci failed",
        _ => "ci note: an earlier verdict failed",
    };
    Some(format!("{lead}: {whose}: {}", record.line()))
}

/// What the CI gate reads, where the row's own line does not already say it: a CI root of
/// the commit's own is that line.
fn ci_line(r: &RowOut) -> Option<String> {
    let ci = r.ci?;
    let note = |by: &jtac_core::GitOid| format!("; {}'s CI root reaches it", short(by));
    let waiver = |root: &str| {
        format!(
            "ci declared_off: a waiver {root} carries{}",
            r.ci_note.as_ref().map(note).unwrap_or_default()
        )
    };
    Some(match (ci, &r.ci_by, r.via) {
        ("failed", ..) => return None,
        (_, Some(by), _) if *by == r.commit => waiver("the commit's own root"),
        (_, Some(by), _) => format!("ci {ci}: {}'s root reaches it", short(by)),
        ("declared_off", None, Some("own")) if r.head.is_none() => waiver("this root"),
        _ => return None,
    })
}
