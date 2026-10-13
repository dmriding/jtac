//! The root: one signed summary per push tip (DESIGN.md section 9.4).

use jiff::Timestamp;
use serde::{Deserialize, Serialize};

use crate::canonical::{CanonicalError, canonical_bytes, canonical_digest};
use crate::digest::Sha256Digest;
use crate::node::{Action, Body, Node};
use crate::oid::GitOid;
use crate::schema::V1;
use crate::signer::Signer;

/// What a push tip's evidence adds up to. Signed as a whole; `levers` are the only nodes
/// that travel with it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Root {
    /// Schema version.
    pub v: V1,
    /// The tip commit.
    pub commit: GitOid,
    /// The tip tree, what the checks judged.
    pub tree: GitOid,
    /// Instant of the verdict and the `now` the verdict function saw.
    pub ts: Timestamp,
    /// Commits between the nearest covered ancestors and the tip.
    pub covers: Covers,
    /// Roots of the nearest covered ancestors. Empty on a non-initial commit is a visible break.
    pub parents: Vec<Parent>,
    /// The policy in force.
    pub policy: PolicyRef,
    /// The gate that produced this root.
    pub gate: GateRef,
    /// Covered, or declared off by a waiver.
    pub state: State,
    /// Merkle root over every node id (DESIGN.md section 9.3).
    pub evidence: Sha256Digest,
    /// Lever nodes in full.
    pub levers: Vec<Node>,
    /// Derived from the nodes. Replay recomputes it; a mismatch invalidates the root.
    pub counts: Counts,
    /// Who signed.
    pub signer: Signer,
}

impl Root {
    /// RFC 8785 bytes, the input to `id` and to the signature.
    pub fn canonical_bytes(&self) -> Result<Vec<u8>, CanonicalError> {
        canonical_bytes(self)
    }

    /// `sha256:` of the canonical bytes.
    pub fn id(&self) -> Result<Sha256Digest, CanonicalError> {
        canonical_digest(self)
    }

    /// Shape rules a reader checks before trusting any field. Signature, Merkle root, and
    /// counts are checked by their own code; this is only what the type cannot express.
    pub fn validate(&self) -> Result<(), RootError> {
        if let Some(index) = self
            .levers
            .iter()
            .position(|n| !matches!(n.body, Body::Lever(_)))
        {
            return Err(RootError::NotALever { index });
        }
        let waived = self
            .levers
            .iter()
            .any(|n| matches!(&n.body, Body::Lever(b) if b.action == Action::Waive));
        if self.state == State::DeclaredOff && !waived {
            return Err(RootError::DeclaredOffWithoutWaiver);
        }
        if let Some(index) = self.levers.iter().position(|n| n.tree != self.tree) {
            return Err(RootError::LeverTree { index });
        }
        Ok(())
    }
}

/// The commits a root vouches for. A list up to the bound, then the ends of the range.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Covers {
    /// Every commit, newest last.
    List(Vec<GitOid>),
    /// Too many to list: the oldest and the tip.
    Range {
        /// Oldest covered commit.
        base: GitOid,
        /// The tip.
        tip: GitOid,
    },
}

/// Link to a covered ancestor's root.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Parent {
    /// The ancestor commit.
    pub commit: GitOid,
    /// Id of its root.
    pub root: Sha256Digest,
}

/// The policy a root was judged under.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PolicyRef {
    /// Hash of the effective policy.
    pub hash: Sha256Digest,
    /// Policy schema version.
    pub version: u32,
    /// Where it came from.
    pub source: PolicySource,
}

/// Where a policy came from. `org` is reserved for v0.2 so v1 roots never need a new reader.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PolicySource {
    /// `.jtac/policy.toml` in the repository.
    Repo,
    /// Built-in defaults, no file present.
    Default,
    /// A signed org policy release.
    Org,
}

/// The gate build that wrote the root.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GateRef {
    /// Gate version string.
    pub version: String,
    /// Digest of the gate binary.
    pub binary: Sha256Digest,
}

/// Coverage state a root can assert. Absent is the missing root, never a value.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum State {
    /// The gate ran and passed.
    Covered,
    /// The gate was waived for the whole commit.
    DeclaredOff,
}

/// Totals derived from the nodes.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Counts {
    /// Check nodes.
    pub checks: u64,
    /// Checks with outcome `pass`.
    pub pass: u64,
    /// Checks with any other outcome.
    pub fail: u64,
    /// Findings by severity.
    pub findings: FindingCounts,
    /// Lever nodes with action `waive`.
    pub waivers: u64,
}

/// Finding totals by severity.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FindingCounts {
    /// Blocker findings.
    pub blocker: u64,
    /// Major findings.
    pub major: u64,
    /// Minor findings.
    pub minor: u64,
}

/// A root whose shape is wrong, whatever its signature says.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RootError {
    /// An entry of `levers` is not a lever node.
    #[error("levers[{index}] is not a lever node")]
    NotALever {
        /// Position in `levers`.
        index: usize,
    },
    /// `declared_off` needs a waiver to point at.
    #[error("state is declared_off but no lever has action waive")]
    DeclaredOffWithoutWaiver,
    /// A lever about some other tree.
    #[error("levers[{index}] is about a different tree than the root")]
    LeverTree {
        /// Position in `levers`.
        index: usize,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::node::tests::{EMPTY, TS, check_node, lever_node};

    fn root() -> Root {
        Root {
            v: V1,
            commit: "sha1:0000000000000000000000000000000000000001"
                .parse()
                .unwrap(),
            tree: GitOid::EMPTY_TREE_SHA1.parse().unwrap(),
            ts: TS.parse().unwrap(),
            covers: Covers::List(vec![
                "sha1:0000000000000000000000000000000000000001"
                    .parse()
                    .unwrap(),
            ]),
            parents: vec![Parent {
                commit: "sha1:0000000000000000000000000000000000000000"
                    .parse()
                    .unwrap(),
                root: EMPTY.parse().unwrap(),
            }],
            policy: PolicyRef {
                hash: EMPTY.parse().unwrap(),
                version: 1,
                source: PolicySource::Repo,
            },
            gate: GateRef {
                version: "0.1.0".into(),
                binary: EMPTY.parse().unwrap(),
            },
            state: State::Covered,
            evidence: EMPTY.parse().unwrap(),
            levers: vec![],
            counts: Counts {
                checks: 1,
                pass: 1,
                ..Counts::default()
            },
            signer: Signer {
                principal: "dev@example.com".into(),
                fingerprint: "SHA256:AAAA".into(),
            },
        }
    }

    #[test]
    fn root_round_trips_with_levers_and_a_range() {
        let mut r = root();
        r.levers.push(lever_node(Action::Waive));
        r.state = State::DeclaredOff;
        r.covers = Covers::Range {
            base: "sha1:0000000000000000000000000000000000000000"
                .parse()
                .unwrap(),
            tip: r.commit,
        };
        let json = serde_json::to_string(&r).unwrap();
        assert_eq!(serde_json::from_str::<Root>(&json).unwrap(), r);
        assert!(
            json.contains(r#""levers":[{"#) && json.contains(r#""base":"#),
            "{json}"
        );
        assert_eq!(
            r.id().unwrap(),
            Sha256Digest::of(r.canonical_bytes().unwrap())
        );
    }

    #[test]
    fn unknown_fields_and_versions_are_refused() {
        let good = serde_json::to_value(root()).unwrap();
        let mut extra = good.clone();
        extra["note"] = "x".into();
        let mut v2 = good.clone();
        v2["v"] = 2.into();
        let mut bad_state = good.clone();
        bad_state["state"] = "absent".into();
        let mut extra_counts = good.clone();
        extra_counts["counts"]["errors"] = 0.into();
        for (name, doc) in [
            ("extra key", extra),
            ("v2", v2),
            ("state absent", bad_state),
            ("extra count", extra_counts),
        ] {
            assert!(
                serde_json::from_value::<Root>(doc).is_err(),
                "{name} was accepted"
            );
        }
    }

    #[test]
    fn validate_table() {
        let mut not_a_lever = root();
        not_a_lever.levers.push(check_node());
        let mut off_without_waiver = root();
        off_without_waiver.state = State::DeclaredOff;
        off_without_waiver.levers.push(lever_node(Action::Init));
        let mut other_tree = root();
        other_tree.levers.push(lever_node(Action::Waive));
        other_tree.levers[0].tree =
            "sha256:6ef19b41225c5369f1c104d45d8d85efa9b057b53b14b4b9b939dd74decc5321"
                .parse()
                .unwrap();
        let mut off_with_waiver = root();
        off_with_waiver.state = State::DeclaredOff;
        off_with_waiver.levers.push(lever_node(Action::Waive));
        let table = [
            (root(), Ok(())),
            (not_a_lever, Err(RootError::NotALever { index: 0 })),
            (off_without_waiver, Err(RootError::DeclaredOffWithoutWaiver)),
            (other_tree, Err(RootError::LeverTree { index: 0 })),
            (off_with_waiver, Ok(())),
        ];
        for (r, want) in table {
            assert_eq!(r.validate(), want);
        }
    }
}
