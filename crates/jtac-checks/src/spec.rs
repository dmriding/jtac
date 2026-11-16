//! One check as the policy names it, in policy order.

use std::time::Duration;

use jtac_policy::{Policy, Source};

/// What the runner needs to run one check.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Spec {
    /// The check id from the policy.
    pub id: String,
    /// The argv to spawn. No shell.
    pub argv: Vec<String>,
    /// How long the check may run before it is killed.
    pub timeout: Duration,
}

/// Every check of `policy` that jtac runs, in the order `jtac_policy::facts` lists them. One
/// read from CI has no argv and is left out.
pub fn specs(policy: &Policy) -> Vec<Spec> {
    let run = |(id, check): (&String, &jtac_policy::Check)| match &check.source {
        Source::Run { argv, timeout_secs } => Some(Spec {
            id: id.clone(),
            argv: argv.clone(),
            timeout: Duration::from_secs(*timeout_secs),
        }),
        Source::Ci(_) => None,
    };
    policy.checks.iter().filter_map(run).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn specs_follow_policy_order_with_argv_and_timeout() {
        let policy = jtac_policy::parse(
            "version = 2\n[checks.test]\nrun = [\"cargo\", \"test\"]\ntimeout_secs = 5\n[checks.fmt]\nrun = [\"cargo\", \"fmt\"]\n[checks.e2e]\nci = { workflow = \".github/workflows/ci.yml\" }\n",
        )
        .unwrap();
        assert_eq!(
            specs(&policy),
            vec![
                Spec {
                    id: "fmt".into(),
                    argv: vec!["cargo".into(), "fmt".into()],
                    timeout: Duration::from_secs(jtac_policy::DEFAULT_TIMEOUT_SECS),
                },
                Spec {
                    id: "test".into(),
                    argv: vec!["cargo".into(), "test".into()],
                    timeout: Duration::from_secs(5),
                },
            ]
        );
    }
}
