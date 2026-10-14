//! The verdict's inputs from a resolved policy (DESIGN.md section 11.2).

use jtac_core::{CanonicalError, CheckSpec, Facts, GitOid};

use crate::hash::policy_hash;
use crate::schema::{INTEGRITY_CHECK_ID, Policy};

/// What `gate` needs from `policy` for `commit` and `tree`. Checks come out in policy order
/// behind the required `integrity` check, which runs first (DESIGN.md section 11.1).
pub fn facts(policy: &Policy, commit: GitOid, tree: GitOid) -> Result<Facts, CanonicalError> {
    Ok(Facts {
        commit,
        tree,
        policy: policy_hash(policy)?,
        checks: std::iter::once(CheckSpec {
            id: INTEGRITY_CHECK_ID.to_owned(),
            required: true,
        })
        .chain(policy.checks.iter().map(|(id, c)| CheckSpec {
            id: id.clone(),
            required: c.required,
        }))
        .collect(),
        block_at: policy.integrity.block_at,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hash::tests::design_example;
    use crate::parse::parse;
    use jtac_core::Severity;

    #[test]
    fn facts_carry_checks_in_policy_order_with_required_and_block_at() {
        let mut toml = String::from(design_example());
        toml.push_str("\n[checks.audit]\nrun = [\"cargo\", \"audit\"]\nrequired = false\n");
        let policy = parse(&toml).unwrap();
        let commit: GitOid = "sha1:0000000000000000000000000000000000000001"
            .parse()
            .unwrap();
        let tree: GitOid = GitOid::EMPTY_TREE_SHA1.parse().unwrap();
        let got = facts(&policy, commit, tree).unwrap();
        assert_eq!((got.commit, got.tree), (commit, tree));
        assert_eq!(got.policy, policy_hash(&policy).unwrap());
        assert_eq!(got.block_at, Severity::Major);
        let ids: Vec<(&str, bool)> = got
            .checks
            .iter()
            .map(|c| (c.id.as_str(), c.required))
            .collect();
        assert_eq!(
            ids,
            [
                ("integrity", true),
                ("audit", false),
                ("fmt", true),
                ("lint", true),
                ("test", true)
            ]
        );
    }
}
