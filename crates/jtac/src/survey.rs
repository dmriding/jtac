//! What `verify` and `audit` share (DESIGN.md section 11.9): the repository, the signers,
//! the range's commits, the roots that bear on them, and the state of each commit as
//! `jtac_gate::rows` assigns it. Both read it; neither decides.

use std::path::Path;

use jtac_core::{GitOid, Sha256Digest, Signer, State};
use jtac_gate::{CommitFact, Row, Trust, Via};
use jtac_store::Store;
use serde::Serialize;

use crate::ledger::{self, Ledger};
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
    let ledger = ledger::gather(&repo, &store, &tool, &signers, &format, &commits)?;
    let facts: Vec<CommitFact> = commits
        .iter()
        .map(|c| CommitFact {
            commit: c.commit,
            tree: c.tree,
        })
        .collect();
    let rows = jtac_gate::rows(&facts, &ledger.roots);
    Ok(Survey {
        range,
        signers,
        ledger,
        rows,
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
                    trust: fact.map(|f| match f.trust {
                        Trust::Trusted => "trusted",
                        _ => "unknown",
                    }),
                    policy_version: fact.map(|f| f.policy_version),
                    gate_version: fact.map(|f| f.gate_version.clone()),
                    invalid: r.invalid.clone(),
                }
            })
            .collect()
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
