//! One check as the policy names it, in policy order.

use std::time::Duration;

use jtac_policy::Policy;

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

/// Every check in `policy`, in the order `jtac_policy::facts` lists them.
pub fn specs(policy: &Policy) -> Vec<Spec> {
    policy
        .checks
        .iter()
        .map(|(id, c)| Spec {
            id: id.clone(),
            argv: c.run.clone(),
            timeout: Duration::from_secs(c.timeout_secs),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn specs_follow_policy_order_with_argv_and_timeout() {
        let policy = jtac_policy::parse(
            "version = 1\n[checks.test]\nrun = [\"cargo\", \"test\"]\ntimeout_secs = 5\n[checks.fmt]\nrun = [\"cargo\", \"fmt\"]\n",
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
