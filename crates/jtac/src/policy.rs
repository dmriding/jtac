//! The policy the verdict runs under: the one committed at HEAD (DESIGN.md decision 30), so
//! the policy hash in every node names rules the tree under test carries.

use std::path::Path;

use jtac_policy::{Matchers, POLICY_PATH, Policy};

use crate::refuse::Refusal;
use crate::repo::{Head, blob_at, on_disk_matches};

/// A parsed policy and whether the work tree holds a different one.
pub struct Loaded {
    /// The policy.
    pub policy: Policy,
    /// Its globs, compiled.
    pub matchers: Matchers,
    /// The file on disk is absent or differs from HEAD's blob after git's clean filters.
    pub differs_on_disk: bool,
}

/// Read and parse HEAD's policy. No file, a non-UTF-8 blob, and a policy that does not parse
/// all refuse.
pub fn load(repo: &Path, head: &Head) -> Result<Loaded, Refusal> {
    let (sha, blob) = blob_at(repo, head, POLICY_PATH)
        .map_err(|e| Refusal::repository(&e))?
        .ok_or_else(Refusal::no_policy)?;
    let text = std::str::from_utf8(&blob).map_err(|_| Refusal::policy_bytes())?;
    let policy = jtac_policy::parse(text).map_err(|e| Refusal::policy(&e))?;
    let matchers = policy.matchers().map_err(|e| Refusal::policy(&e))?;
    let same = on_disk_matches(repo, POLICY_PATH, &sha).map_err(|e| Refusal::repository(&e))?;
    Ok(Loaded {
        policy,
        matchers,
        differs_on_disk: !same,
    })
}
