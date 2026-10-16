//! `jtac verify` (DESIGN.md sections 7.1 and 11.9): the roots that bear on a range, judged;
//! exit 1 when any is invalid, or with `--require-coverage` when any commit is absent.

use std::fmt::Write;
use std::path::Path;

use jtac_checks::git::quoted;
use jtac_core::{GitOid, Sha256Digest};
use serde::Serialize;

use crate::Printed;
use crate::cli::VerifyArgs;
use crate::refuse::Refusal;
use crate::survey::{WaiverRow, survey};

/// What `jtac verify` prints.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct VerifyOut {
    /// Output schema version.
    pub v: u32,
    /// The command.
    pub command: &'static str,
    /// The range as listed.
    pub range: Vec<String>,
    /// Where trust came from.
    pub signers: String,
    /// Commits in the range.
    pub commits: u64,
    /// Covered, by any route.
    pub covered: u64,
    /// Declared off, by any route.
    pub declared_off: u64,
    /// Neither.
    pub absent: u64,
    /// Absent commits, newest first; only listed when `--require-coverage` fails on them.
    pub absent_commits: Vec<GitOid>,
    /// Roots that bear on the range.
    pub roots: u64,
    /// Those that do not count, with why.
    pub invalid: Vec<Invalid>,
    /// Those whose signer, or a waiver's signer, no trust file names.
    pub unknown_signers: u64,
    /// Every waiver a valid root applies.
    pub waivers: Vec<WaiverRow>,
    /// Those written under an agent session.
    pub agent_waivers: u64,
    /// Whether absence fails.
    pub require_coverage: bool,
    /// Skipped refs, parent roots this clone lacks.
    pub notes: Vec<String>,
    /// Exit 0 when true.
    pub pass: bool,
}

/// A root that does not count.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Invalid {
    /// The commit its ref is for.
    pub commit: GitOid,
    /// The root id, when the root could be read.
    pub root: Option<Sha256Digest>,
    /// Why.
    pub why: String,
}

/// Gather and render.
pub fn verify(a: &VerifyArgs, cwd: &Path) -> Result<Printed, Refusal> {
    let s = survey(cwd, "verify", &a.range, a.signers.as_deref())?;
    let (covered, declared_off, absent) = s.totals();
    let invalid: Vec<Invalid> = s
        .ledger
        .roots
        .iter()
        .filter_map(|r| {
            r.invalid.as_ref().map(|why| Invalid {
                commit: r.commit,
                root: r.id,
                why: why.clone(),
            })
        })
        .collect();
    let absent_commits = if a.require_coverage {
        s.rows
            .iter()
            .filter(|r| r.state.is_none())
            .map(|r| r.commit)
            .collect()
    } else {
        Vec::new()
    };
    let pass = invalid.is_empty() && absent_commits.is_empty();
    let waivers = s.waivers_out();
    let out = VerifyOut {
        v: 1,
        command: "verify",
        range: s.range.clone(),
        signers: s.signers.describe(),
        commits: s.rows.len() as u64,
        covered,
        declared_off,
        absent,
        absent_commits,
        roots: s.ledger.roots.len() as u64,
        unknown_signers: s.unknown_signers(),
        agent_waivers: waivers.iter().filter(|w| w.waiver.agent.is_some()).count() as u64,
        waivers,
        invalid,
        require_coverage: a.require_coverage,
        notes: s.ledger.notes,
        pass,
    };
    let text = if a.json {
        crate::render::json(&out)?
    } else {
        render(&out)
    };
    Ok(Printed::verdict(text, out.pass))
}

fn render(o: &VerifyOut) -> String {
    let mut s = String::new();
    let _ = writeln!(
        s,
        "{} {}: {} commits, {} covered, {} declared off, {} absent; {} roots checked, {} \
         invalid, {} signed by unknown keys",
        if o.pass { "VERIFIED" } else { "INVALID" },
        o.range.join(" "),
        o.commits,
        o.covered,
        o.declared_off,
        o.absent,
        o.roots,
        o.invalid.len(),
        o.unknown_signers
    );
    let _ = writeln!(s, "  signers {}", o.signers);
    for w in &o.waivers {
        let _ = writeln!(s, "  waiver  {}", w.line());
    }
    for i in &o.invalid {
        let _ = writeln!(s, "  invalid {} {}", i.commit.hex(), i.why);
    }
    for c in &o.absent_commits {
        let _ = writeln!(s, "  absent  {}", c.hex());
    }
    if !o.absent_commits.is_empty() {
        let _ = writeln!(
            s,
            "  fix     run `jtac check` on each absent commit, or `jtac audit {}` for the \
             table",
            o.range
                .iter()
                .map(|w| quoted(w, "RANGE"))
                .collect::<Vec<_>>()
                .join(" ")
        );
    }
    for note in &o.notes {
        let _ = writeln!(s, "  note    {note}");
    }
    s
}
