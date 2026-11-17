//! The verdict's inputs from a resolved policy (DESIGN.md section 11.2).

use jtac_core::{CanonicalError, CheckSpec, Facts, GitOid};

use crate::hash::policy_hash;
use crate::schema::{INTEGRITY_CHECK_ID, Policy};

/// Which checks a gate's facts hold (DESIGN.md section 15.2). The one statement of the
/// rule, for every gate that judges and every reader that judges again.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Checks {
    /// The `run` checks alone: every gate but the merge job, which leaves each `ci` check
    /// unread.
    Run,
    /// Every check, each `ci` one with its pin: the merge job, whose root has `from`.
    All,
}

impl Checks {
    /// By the shape of the root a gate writes or a reader holds.
    pub fn of(from: bool) -> Self {
        if from { Self::All } else { Self::Run }
    }

    /// The `ci` checks of `policy` this gate did not read, in policy order: a root's
    /// `unread`, derived here and chosen nowhere.
    pub fn unread(self, policy: &Policy) -> Vec<String> {
        match self {
            Self::Run => policy.ci_checks().map(|(id, _)| id.to_owned()).collect(),
            Self::All => Vec::new(),
        }
    }
}

/// What `gate` needs from `policy` for `commit` and `tree`. Checks come out in policy order
/// behind the required `integrity` check, which runs first (DESIGN.md section 11.1), each with
/// its pin when CI holds its result; `checks` says whether those are among them.
pub fn facts(
    policy: &Policy,
    commit: GitOid,
    tree: GitOid,
    checks: Checks,
) -> Result<Facts, CanonicalError> {
    let held = |c: &&crate::Check| checks == Checks::All || c.pin().is_none();
    Ok(Facts {
        commit,
        tree,
        policy: policy_hash(policy)?,
        checks: std::iter::once(CheckSpec {
            id: INTEGRITY_CHECK_ID.to_owned(),
            required: true,
            pin: None,
        })
        .chain(
            (policy.checks.iter())
                .filter(|(_, c)| held(c))
                .map(|(id, c)| CheckSpec {
                    id: id.clone(),
                    required: c.required,
                    pin: c.pin().cloned(),
                }),
        )
        .collect(),
        block_at: policy.integrity.block_at,
        waivable: Vec::new(),
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
        let mut toml = design_example().replace("version = 1", "version = 2");
        toml.push_str("\n[checks.audit]\nrun = [\"cargo\", \"audit\"]\nrequired = false\n");
        toml.push_str("[checks.e2e]\nci = { workflow = \".github/workflows/ci.yml\" }\n");
        let policy = parse(&toml).unwrap();
        let commit: GitOid = "sha1:0000000000000000000000000000000000000001"
            .parse()
            .unwrap();
        let tree: GitOid = GitOid::EMPTY_TREE_SHA1.parse().unwrap();
        let got = facts(&policy, commit, tree, Checks::All).unwrap();
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
                ("e2e", true),
                ("fmt", true),
                ("lint", true),
                ("test", true)
            ]
        );
        let pins: Vec<_> = (got.checks.iter())
            .filter_map(|c| Some((c.id.as_str(), c.pin.as_ref()?.workflow.as_str())))
            .collect();
        assert_eq!(pins, [("e2e", ".github/workflows/ci.yml")]);
    }

    /// Section 15.2: a root without `from` judged the `run` checks and names the rest; one
    /// with `from` read every check and names none. The hash is the whole policy's either way.
    #[test]
    fn a_gate_without_from_holds_the_run_checks_and_lists_each_ci_check_unread() {
        let mut toml = design_example().replace("version = 1", "version = 2");
        toml.push_str("\n[checks.e2e]\nci = { workflow = \".github/workflows/ci.yml\" }\n");
        toml.push_str(
            "[checks.acc]\nci = { workflow = \".github/workflows/ci.yml\", job = \"a\" }\n",
        );
        let policy = parse(&toml).unwrap();
        let commit: GitOid = "sha1:0000000000000000000000000000000000000001"
            .parse()
            .unwrap();
        let tree: GitOid = GitOid::EMPTY_TREE_SHA1.parse().unwrap();
        let ids = |checks: Checks| -> Vec<String> {
            let got = facts(&policy, commit, tree, checks).unwrap();
            assert_eq!(got.policy, policy_hash(&policy).unwrap());
            got.checks.into_iter().map(|c| c.id).collect()
        };
        assert_eq!(
            (Checks::of(false), Checks::of(true)),
            (Checks::Run, Checks::All)
        );
        assert_eq!(ids(Checks::Run), ["integrity", "fmt", "lint", "test"]);
        assert_eq!(
            ids(Checks::All),
            ["integrity", "acc", "e2e", "fmt", "lint", "test"]
        );
        assert_eq!(Checks::Run.unread(&policy), ["acc", "e2e"]);
        assert!(Checks::All.unread(&policy).is_empty());
        let plain = parse(design_example()).unwrap();
        assert!(Checks::Run.unread(&plain).is_empty());
    }
}
