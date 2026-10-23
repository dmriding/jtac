//! The comparison of a head's root with this gate's own run of the root's diff (DESIGN.md
//! section 15, decision 42). Pure: both sides come in as facts. Signature, tree, shape and
//! chain are judged before this and a failure there is `Invalid`; history the clone lacks is
//! `Unverifiable`; both are the caller's to say. Nodes never travel, so state and counts are
//! the comparison that exists, and only under one gate version and one policy hash.

use serde::{Deserialize, Serialize};

use crate::digest::Sha256Digest;
use crate::root::{Counts, Root, State};
use crate::schema::RootVersion;

/// What a merge job made of a head's root; a merge root records it in `from`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Compared {
    /// Same gate version, same policy hash, same state and counts.
    Agreed,
    /// Same gate version and policy hash; state or counts differ.
    Disagreed,
    /// Same gate version; the root names a policy hash this gate does not compute for the
    /// policy it should have judged under.
    Lied,
    /// Unreadable, wrong tree, a signature that does not hold, or a shape or chain rule
    /// broken; the caller's finding.
    Invalid,
    /// Another gate version, version 1, or history the clone lacks: nothing to compare.
    Unverifiable,
}

/// This gate's side: its version, the hash it computes for the policy the root should have
/// judged under, and what its run from the root's own base found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Run {
    /// `gate.version` of this build.
    pub gate_version: String,
    /// The policy hash this build computes.
    pub policy: Sha256Digest,
    /// The state the run reached.
    pub state: State,
    /// The counts the run reached.
    pub counts: Counts,
}

/// Agreed, disagreed, lied or unverifiable; never `Invalid`, which is decided before.
pub fn compare(root: &Root, run: &Run) -> Compared {
    if root.v == RootVersion::V1 || root.gate.version != run.gate_version {
        return Compared::Unverifiable;
    }
    if root.policy.hash != run.policy {
        return Compared::Lied;
    }
    if root.state != run.state || root.counts != run.counts {
        return Compared::Disagreed;
    }
    Compared::Agreed
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::root::tests::v2;

    fn run(root: &Root) -> Run {
        Run {
            gate_version: root.gate.version.clone(),
            policy: root.policy.hash,
            state: root.state,
            counts: root.counts.clone(),
        }
    }

    #[test]
    fn compare_answers_by_version_then_hash_then_state_and_counts() {
        let root = v2();
        let other_hash: Sha256Digest =
            "sha256:0000000000000000000000000000000000000000000000000000000000000000"
                .parse()
                .unwrap();
        let mut v1 = v2();
        v1.v = RootVersion::V1;
        v1.base = None;
        let table = [
            ("same", run(&root), Compared::Agreed),
            (
                "other gate version",
                Run {
                    gate_version: "0.2.0".into(),
                    ..run(&root)
                },
                Compared::Unverifiable,
            ),
            (
                "other policy hash",
                Run {
                    policy: other_hash,
                    ..run(&root)
                },
                Compared::Lied,
            ),
            (
                "other state",
                Run {
                    state: State::DeclaredOff,
                    ..run(&root)
                },
                Compared::Disagreed,
            ),
            (
                "other counts",
                Run {
                    counts: Counts {
                        checks: 1,
                        ..Counts::default()
                    },
                    ..run(&root)
                },
                Compared::Disagreed,
            ),
        ];
        for (name, side, want) in table {
            assert_eq!(compare(&root, &side), want, "{name}");
        }
        assert_eq!(compare(&v1, &run(&v1)), Compared::Unverifiable);
        assert_eq!(
            serde_json::to_string(&Compared::Unverifiable).unwrap(),
            "\"unverifiable\""
        );
    }
}
