//! What `verify` and `audit` share (DESIGN.md section 11.9): the repository, the signers,
//! the range's commits, the roots that bear on them, and the state of each commit as
//! `jtac_gate::rows` assigns it. Both read it; neither decides.

use std::path::Path;

use jtac_core::{GitOid, Sha256Digest, Signer, State};
use jtac_gate::{CommitFact, Row, Trust, Via};
use jtac_store::Store;
use serde::Serialize;

use crate::headof::{self, HeadOut};
use crate::ledger::{self, Ledger};
use crate::output::WaiverOut;
use crate::range;
use crate::refuse::Refusal;
use crate::trust::Signers;
use crate::{coverage, repo};

/// Everything gathered for one range.
pub struct Survey {
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

/// Gather for `command`. An empty `range` is `HEAD`.
pub fn survey(
    cwd: &Path,
    command: &str,
    range: &[String],
    signers: Option<&Path>,
) -> Result<Survey, Refusal> {
    let repo = repo::root(cwd).map_err(|e| Refusal::repository(&e))?;
    if let Some(path) = repo::grafted(&repo).map_err(|e| Refusal::repository(&e))? {
        return Err(Refusal::grafted(&path));
    }
    let format = repo::format(&repo).map_err(|e| Refusal::repository(&e))?;
    let head = repo::head(&repo).map_err(|e| Refusal::no_head(&e))?;
    let store = Store::open(&repo).map_err(Refusal::store)?;
    let tool = coverage::verifier(&repo)?;
    let signers = Signers::gather(&repo, &head, signers)?;
    let range: Vec<String> = if range.is_empty() {
        vec!["HEAD".to_owned()]
    } else {
        range.to_vec()
    };
    let commits = range::commits(&repo, &format, command, &range)?;
    let ledger = ledger::gather(&repo, &store, &tool, &signers, command, &format, &commits)?;
    let facts: Vec<CommitFact> = commits
        .iter()
        .map(|c| CommitFact {
            commit: c.commit,
            tree: c.tree,
        })
        .collect();
    let rows = jtac_gate::rows(&facts, &ledger.roots);
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
    /// `covered`, `declared_off`, or `absent`.
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

/// The name a state prints as.
pub fn state_name(state: Option<State>) -> &'static str {
    match state {
        Some(State::Covered) => "covered",
        Some(State::DeclaredOff) => "declared_off",
        None => "absent",
    }
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
                    state: state_name(r.state),
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
                }
            })
            .collect()
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

    /// Commits by state: covered, declared off, absent.
    pub fn totals(&self) -> (u64, u64, u64) {
        let count =
            |want: Option<State>| self.rows.iter().filter(|r| r.state == want).count() as u64;
        (
            count(Some(State::Covered)),
            count(Some(State::DeclaredOff)),
            count(None),
        )
    }
}

/// The line that names which root gave a row its state.
pub fn via_text(row: &RowOut) -> String {
    match (row.via, row.by) {
        (Some("own"), _) => "own root".to_owned(),
        (Some("covers"), Some(by)) => format!("covered by {}'s root", short(&by)),
        (Some("tree"), Some(by)) => format!("by tree, {}'s root", short(&by)),
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
