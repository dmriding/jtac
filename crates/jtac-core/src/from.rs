//! The head a merge root stands for (DESIGN.md section 15): the pull request head commit,
//! its root when the merge job found one, and what the job made of that root, so what the
//! head had at merge time is inside the signed evidence and a root published later changes
//! nothing.

use serde::{Deserialize, Serialize};

use crate::compare::Compared;
use crate::digest::Sha256Digest;
use crate::oid::GitOid;

/// The `from` field of a version 2 merge root.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FromHead {
    /// The pull request head commit.
    pub head: GitOid,
    /// The head root's id, when the job found one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub root: Option<Sha256Digest>,
    /// What the job made of that root; present exactly when `root` is.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compared: Option<Compared>,
}
