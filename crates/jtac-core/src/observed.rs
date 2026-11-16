//! A check whose result was read from the team's CI: node version 3's body (DESIGN.md
//! sections 9.2 and 15.2).

use serde::{Deserialize, Serialize};

use crate::node::Outcome;
use crate::oid::GitOid;
use crate::pin::Pin;

/// One `ci` check as the merge job read it: `id` and `outcome` as a run's, and where the
/// result came from in place of every field a run has.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ObservedCheck {
    /// The check id from the policy.
    pub id: String,
    /// What the forge's word maps to.
    pub outcome: Outcome,
    /// The run the result was read from.
    pub observed: Observed,
}

/// The source of an observed result, kept so a later reader can see what was read.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Observed {
    /// The forge that reported the run.
    pub forge: Forge,
    /// The definition file's path in the tree, as policy names it.
    pub workflow: String,
    /// That file's blob in the commit's tree, read from git and never from the forge.
    pub blob: GitOid,
    /// The job read; absent when policy names none and the run's conclusion was the result.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub job: Option<String>,
    /// The commit the run was for.
    pub commit: GitOid,
    /// The forge's id for the run.
    pub run: u64,
    /// The attempt read, the newest the forge listed.
    pub attempt: u64,
    /// The forge's own word for how the job or the run ended.
    pub conclusion: String,
}

impl Observed {
    /// Whether this is a result for `pin` on `commit`, the only one the verdict counts
    /// (`a_ci_check_counts_only_a_version_3_node_for_its_pin_and_commit`).
    pub fn is_for(&self, pin: &Pin, commit: &GitOid) -> bool {
        self.workflow == pin.workflow && self.job == pin.job && self.commit == *commit
    }
}

/// A forge a result is read from. A reader refuses one it has no mapping for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Forge {
    /// GitHub Actions.
    Github,
}

impl Forge {
    /// What this forge's `word` for a conclusion maps to: the one mapping, for selection
    /// and for the node's reader. Every word but four is `error`, so a check that did not
    /// run never looks like a pass (`every_conclusion_but_four_is_an_error`).
    pub fn outcome(self, word: &str) -> Outcome {
        match (self, word) {
            (Self::Github, "success") => Outcome::Pass,
            (Self::Github, "failure" | "timed_out") => Outcome::Fail,
            (Self::Github, "skipped") => Outcome::Skipped,
            (Self::Github, _) => Outcome::Error,
        }
    }
}

impl ObservedCheck {
    /// Whether `outcome` is what the forge's word says, or `error`, which a job name held
    /// twice reads under any word. The node's reader refuses the rest
    /// (`a_version_3_node_says_what_its_conclusion_says_or_error`).
    pub fn coherent(&self) -> bool {
        let read = self.observed.forge.outcome(&self.observed.conclusion);
        self.outcome == read || self.outcome == Outcome::Error
    }
}
