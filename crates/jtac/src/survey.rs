//! What `verify` and `audit` share (DESIGN.md section 11.9): the repository, the signers,
//! the range's commits, the roots that bear on them, and the state of each commit as
//! `jtac_gate::rows` assigns it. Both read it; neither decides.

use std::path::Path;

use jtac_core::{Body, Counts, GitOid, ObservedCheck, Outcome, Sha256Digest, Signer, State};
use jtac_gate::{Ci, CommitFact, PolicyKind, Reading, Row, Trust, Via};
use jtac_store::Store;
use serde::Serialize;

use crate::headof::{self, HeadOut};
use crate::ledger::{self, Ledger};
use crate::output::{WaiverOut, plain};
use crate::range;
use crate::reason::source;
use crate::refuse::Refusal;
use crate::trust::Signers;
use crate::{coverage, repo};

/// Everything gathered for one range.
pub struct Survey {
    /// The repository.
    pub repo: std::path::PathBuf,
    /// The range as given, or the default.
    pub range: Vec<String>,
    /// Where trust came from.
    pub signers: Signers,
    /// The roots that bear on them.
    pub ledger: Ledger,
    /// One per commit, in `commits` order.
    pub rows: Vec<Row>,
    /// One per root, in `ledger.roots` order: what a valid root with `from` says of its head.
    pub heads: Vec<Option<HeadOut>>,
}

/// Gather for `command`. An empty `range` is `HEAD`; trust is HEAD's tree, or `at`'s, shown
/// by its name, when a command stands on another commit (a release reads the tag's). A commit
/// whose policy the ledger did not ask is given `NotHeld`: no reading consults it, and one
/// that did would refuse, never pass.
pub fn survey(
    cwd: &Path,
    command: &str,
    range: &[String],
    signers: Option<&Path>,
    at: Option<(&repo::Head, &str)>,
) -> Result<Survey, Refusal> {
    let repo = repo::root(cwd).map_err(|e| Refusal::repository(&e))?;
    if let Some(path) = repo::grafted(&repo).map_err(|e| Refusal::repository(&e))? {
        return Err(Refusal::grafted(&path));
    }
    let format = repo::format(&repo).map_err(|e| Refusal::repository(&e))?;
    let head = repo::head(&repo).map_err(|e| Refusal::no_head(&e))?;
    let store = Store::open(&repo).map_err(Refusal::store)?;
    let tool = coverage::verifier(&repo)?;
    let signers = match (signers, at) {
        (Some(path), _) => Signers::File(path.to_path_buf()),
        (None, Some((at, shown))) => Signers::at(&repo, at, shown)?,
        (None, None) => Signers::gather(&repo, &head, None)?,
    };
    let range: Vec<String> = if range.is_empty() {
        vec!["HEAD".to_owned()]
    } else {
        range.to_vec()
    };
    let commits = range::commits(&repo, &format, command, &range)?;
    let site = ledger::Site {
        repo: &repo,
        store: &store,
        tool: &tool,
        signers: &signers,
        command,
        format: &format,
    };
    let mut ledger = ledger::gather(&site, &commits)?;
    ledger.read_records(&site, &commits)?;
    let facts: Vec<CommitFact> = commits
        .iter()
        .map(|c| CommitFact {
            commit: c.commit,
            tree: c.tree,
            policy: (ledger.policies.get(&c.tree)).map_or(PolicyKind::NotHeld, |p| p.kind()),
        })
        .collect();
    let rows = jtac_gate::rows(&facts, &ledger.roots, &ledger.records);
    let mut heads = Vec::new();
    for (fact, stored) in ledger.roots.iter().zip(&ledger.stored) {
        let from = stored
            .as_ref()
            .filter(|_| fact.invalid.is_none())
            .and_then(|s| s.root.from.as_ref());
        heads.push(match from {
            Some(from) => Some(headof::head_of(&store, &tool, &signers, command, from)?),
            None => None,
        });
    }
    Ok(Survey {
        repo,
        range,
        signers,
        ledger,
        rows,
        heads,
    })
}

/// One commit as `--json` shows it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct RowOut {
    /// The commit.
    pub commit: GitOid,
    /// `covered`, `declared_off`, `partial` (the local gate passed and left a `ci` check
    /// unread), `policy_not_held`, or `absent`.
    pub state: &'static str,
    /// `own`, `covers`, or `tree`; absent has none.
    pub via: Option<&'static str>,
    /// The commit whose root gave the state, when not the commit's own.
    pub by: Option<GitOid>,
    /// That root.
    pub root: Option<Sha256Digest>,
    /// Who signed it.
    pub signer: Option<Signer>,
    /// `trusted` or `unknown`.
    pub trust: Option<&'static str>,
    /// `policy.version` of that root.
    pub policy_version: Option<u32>,
    /// `gate.version` of that root.
    pub gate_version: Option<String>,
    /// Why the commit's own root did not count, when it has one that did not.
    pub invalid: Option<String>,
    /// For a merge root of the commit's own: its head (DESIGN.md section 15).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub head: Option<HeadOut>,
    /// What the CI gate reads (DESIGN.md section 9.5): `covered`, `declared_off` or
    /// `failed`; absent when no document of that gate's counts for the commit.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ci: Option<&'static str>,
    /// The commit whose root gave the CI gate's reading, when not this row's root.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ci_by: Option<GitOid>,
    /// Beside a CI waiver: the commit whose CI root reaches this one from a later tip.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ci_note: Option<GitOid>,
    /// The failure record that reaches the commit (DESIGN.md section 15.2): what `failed`
    /// was read from, or the note beside a reading that outranks it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ci_record: Option<RecordOut>,
}

/// A failure record as a row shows it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct RecordOut {
    /// The commit whose verdict failed: the row's own, or a later tip's that reaches it.
    pub commit: GitOid,
    /// The record's id.
    pub record: Sha256Digest,
    /// Each check read from CI that did not pass, with the forge's word.
    pub observed: Vec<ObservedCheck>,
    /// What the failed verdict counted.
    pub counts: Counts,
}

impl RecordOut {
    /// What failed, every stored word plain: the checks read from CI that did not pass,
    /// then the counts, which is all a record holds of a check the job ran.
    pub fn line(&self) -> String {
        let mut s = String::new();
        for c in &self.observed {
            s.push_str(&format!(
                "check `{}` {}, CI concluded `{}` ({}); ",
                plain(&c.id),
                c.outcome,
                plain(&c.observed.conclusion),
                source(&c.observed)
            ));
        }
        s + &crate::ci::out::counts(&self.counts)
    }
}

/// One waiver a valid root in the range applies, as shown.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct WaiverRow {
    /// The commit whose root applies it.
    pub commit: GitOid,
    /// The waiver.
    #[serde(flatten)]
    pub waiver: WaiverOut,
    /// `trusted` or `unknown`: what the trust file said about the waiver's signer.
    pub trust: &'static str,
}

impl WaiverRow {
    /// One line: the commit, then the waiver's own line, then trust.
    pub fn line(&self) -> String {
        format!(
            "{} {} ({})",
            short(&self.commit),
            self.waiver.line(),
            self.trust
        )
    }
}

fn trust_name(t: &Trust) -> &'static str {
    match t {
        Trust::Trusted => "trusted",
        _ => "unknown",
    }
}

/// The name a root's state prints as.
pub fn state_name(state: Option<State>) -> &'static str {
    match state {
        Some(State::Covered) => "covered",
        Some(State::DeclaredOff) => "declared_off",
        Some(State::Failed) => "failed",
        None => "absent",
    }
}

/// The name a row's reading prints as in `--json`.
pub fn reading_name(state: Option<Reading>) -> &'static str {
    match state {
        Some(Reading::Covered) => "covered",
        Some(Reading::DeclaredOff) => "declared_off",
        Some(Reading::Partial) => "partial",
        Some(Reading::NotHeld) => "policy_not_held",
        None => "absent",
    }
}

/// Commits by reading. Only `covered` and `declared_off` count as covered.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Tally {
    /// Covered, by any route.
    pub covered: u64,
    /// Declared off, by any route.
    pub declared_off: u64,
    /// The local gate's root alone, a `ci` check unread.
    pub partial: u64,
    /// A policy the reading turns on is not in this clone.
    pub not_held: u64,
    /// No valid root bears.
    pub absent: u64,
}

impl Survey {
    /// The rows as shown.
    pub fn rows_out(&self) -> Vec<RowOut> {
        self.rows
            .iter()
            .map(|r| {
                let fact = r.root.map(|i| &self.ledger.roots[i]);
                RowOut {
                    commit: r.commit,
                    state: reading_name(r.state),
                    via: r.via.as_ref().map(|v| match v {
                        Via::Own => "own",
                        Via::Covers { .. } => "covers",
                        Via::Tree { .. } => "tree",
                    }),
                    by: match &r.via {
                        Some(Via::Covers { tip }) => Some(*tip),
                        Some(Via::Tree { commit }) => Some(*commit),
                        _ => None,
                    },
                    root: fact.and_then(|f| f.id),
                    signer: fact.and_then(|f| f.signer.clone()),
                    trust: fact.map(|f| trust_name(&f.trust)),
                    policy_version: fact.map(|f| f.policy_version),
                    gate_version: fact.map(|f| f.gate_version.clone()),
                    invalid: r.invalid.clone(),
                    head: match (&r.via, r.root) {
                        (Some(Via::Own), Some(i)) => self.heads[i].clone(),
                        _ => None,
                    },
                    ci: match r.ci {
                        Ci::Absent => None,
                        Ci::Covered { .. } => Some("covered"),
                        Ci::DeclaredOff { .. } => Some("declared_off"),
                        Ci::Failed { .. } => Some("failed"),
                    },
                    ci_by: (self.ci_root(r).filter(|i| Some(*i) != r.root))
                        .map(|i| self.ledger.roots[i].commit),
                    ci_note: match r.ci {
                        Ci::DeclaredOff { note, .. } => note.map(|i| self.ledger.roots[i].commit),
                        _ => None,
                    },
                    ci_record: r.record.and_then(|i| self.record_out(i)),
                }
            })
            .collect()
    }

    /// The failure record at `index` as shown; `None` only for a record with no id.
    fn record_out(&self, index: usize) -> Option<RecordOut> {
        let root = &self.ledger.failed[index].root;
        let failing = |n: &jtac_core::Node| match &n.body {
            Body::Observed(c) if c.outcome != Outcome::Pass => Some(c.clone()),
            _ => None,
        };
        Some(RecordOut {
            commit: root.commit,
            record: root.id().ok()?,
            observed: root.observed.iter().filter_map(failing).collect(),
            counts: root.counts.clone(),
        })
    }

    /// The root the CI gate read `row` through, when it read one.
    pub fn ci_root(&self, row: &Row) -> Option<usize> {
        match row.ci {
            Ci::Covered { root } | Ci::DeclaredOff { root, .. } => Some(root),
            Ci::Absent | Ci::Failed { .. } => None,
        }
    }

    /// Refuse when a row turns on a policy this clone does not hold (decision 60): a
    /// command that judges answers 2 with the fetch named, where a report prints the row.
    pub fn held(&self, command: &str) -> Result<(), Refusal> {
        let trees = self.rows.iter().flat_map(|r| &r.lacking);
        self.ledger.held(&self.repo, command, trees)
    }

    /// Every waiver a valid root applies, in root order.
    pub fn waivers_out(&self) -> Vec<WaiverRow> {
        let mut out = Vec::new();
        for (fact, stored) in self.ledger.roots.iter().zip(&self.ledger.stored) {
            let Some(s) = stored.as_ref().filter(|_| fact.invalid.is_none()) else {
                continue;
            };
            for (lever, trust) in s.root.levers.iter().zip(&fact.waiver_trust) {
                if let Some(waiver) = WaiverOut::of(lever) {
                    out.push(WaiverRow {
                        commit: fact.commit,
                        waiver,
                        trust: trust_name(trust),
                    });
                }
            }
        }
        out
    }

    /// Valid roots whose signer, or any of whose waivers' signers, no trust file names.
    pub fn unknown_signers(&self) -> u64 {
        let unknown = |t: &Trust| !matches!(t, Trust::Trusted);
        self.ledger
            .roots
            .iter()
            .filter(|r| {
                r.invalid.is_none() && (unknown(&r.trust) || r.waiver_trust.iter().any(unknown))
            })
            .count() as u64
    }

    /// Commits by reading.
    pub fn totals(&self) -> Tally {
        let mut t = Tally::default();
        for r in &self.rows {
            *match r.state {
                Some(Reading::Covered) => &mut t.covered,
                Some(Reading::DeclaredOff) => &mut t.declared_off,
                Some(Reading::Partial) => &mut t.partial,
                Some(Reading::NotHeld) => &mut t.not_held,
                None => &mut t.absent,
            } += 1;
        }
        t
    }
}

/// The line that names which root gave a row its state.
pub fn via_text(row: &RowOut) -> String {
    match (row.via, row.by) {
        (Some("own"), _) => "own root".to_owned(),
        (Some("covers"), Some(by)) => format!("covered by {}'s root", short(&by)),
        (Some("tree"), Some(by)) => format!("by tree, {}'s root", short(&by)),
        _ if row.state == "policy_not_held" => "a policy it turns on is not in this clone".into(),
        _ => match &row.invalid {
            Some(why) => format!("root invalid: {why}"),
            None => "no root".to_owned(),
        },
    }
}

/// The first twelve hex digits.
pub fn short(oid: &GitOid) -> String {
    oid.hex()[..12].to_owned()
}
