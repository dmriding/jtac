//! The root: one signed summary per push tip (DESIGN.md section 9.4).

use serde::{Deserialize, Serialize};

use crate::canonical::{CanonicalError, canonical_bytes, canonical_digest};
use crate::digest::Sha256Digest;
use crate::node::{Action, Body, Node, Scope};
use crate::oid::GitOid;
use crate::schema::V1;
use crate::signer::Signer;
use crate::time::Ts;

/// What a push tip's evidence adds up to. Signed as a whole; applied waivers are the only
/// nodes that travel with it.
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
    pub ts: Ts,
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
    /// Every waiver the verdict applied, with its signature.
    pub levers: Vec<SignedLever>,
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
        for (index, lever) in self.levers.iter().enumerate() {
            let Body::Lever(body) = &lever.node.body else {
                return Err(RootError::NotAWaiver { index });
            };
            if body.action != Action::Waive {
                return Err(RootError::NotAWaiver { index });
            }
            match &body.scope {
                None => return Err(RootError::WaiverScope { index }),
                Some(Scope::Commit { commit }) if *commit != self.commit => {
                    return Err(RootError::WaiverCommit { index });
                }
                Some(Scope::Rule { .. }) if body.expires.is_none() => {
                    return Err(RootError::WaiverExpiry { index });
                }
                Some(_) => {}
            }
        }
        if self.state == State::DeclaredOff && !self.levers.iter().any(|l| l.covers_commit()) {
            return Err(RootError::DeclaredOffWithoutWaiver);
        }
        let tip = match &self.covers {
            Covers::List(list) => list.last(),
            Covers::Range { tip, .. } => Some(tip),
        };
        if tip != Some(&self.commit) {
            return Err(RootError::CoversTip);
        }
        Ok(())
    }
}

/// A waiver and the signature over its node, namespace `jtac-lever-v1` (DESIGN.md 11.8).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SignedLever {
    /// The lever node in full.
    pub node: Node,
    /// SSHSIG over the node's canonical bytes.
    pub sig: String,
}

impl SignedLever {
    fn covers_commit(&self) -> bool {
        matches!(&self.node.body, Body::Lever(b) if matches!(b.scope, Some(Scope::Commit { .. })))
    }
}

/// The commits a root vouches for, oldest first and ending at the tip. Past the bound, the
/// ends of the range.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged, deny_unknown_fields)]
pub enum Covers {
    /// Every commit.
    List(Vec<GitOid>),
    /// Too many to list.
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
    /// A waiver covers the whole commit.
    DeclaredOff,
}

/// Totals derived from the nodes (DESIGN.md section 9.4).
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
    /// Length of `levers`.
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

/// A root whose shape is wrong, whatever its signature says. `index` is the position in
/// `levers`.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RootError {
    /// An entry of `levers` is not a lever node with action `waive`.
    #[error("levers[{index}] is not a waiver")]
    NotAWaiver {
        /// Where.
        index: usize,
    },
    /// A waiver with no scope.
    #[error("levers[{index}] has no scope")]
    WaiverScope {
        /// Where.
        index: usize,
    },
    /// A commit-scoped waiver for some other commit.
    #[error("levers[{index}] waives a different commit than the root's")]
    WaiverCommit {
        /// Where.
        index: usize,
    },
    /// A rule-scoped waiver with no expiry (DESIGN.md section 11.6).
    #[error("levers[{index}] is a rule waiver without an expiry")]
    WaiverExpiry {
        /// Where.
        index: usize,
    },
    /// `declared_off` needs a waiver that covers the whole commit.
    #[error("state is declared_off but no lever waives the whole commit")]
    DeclaredOffWithoutWaiver,
    /// `covers` does not end at the root's commit.
    #[error("covers does not end at the root's commit")]
    CoversTip,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::canonical::from_canonical;
    use crate::node::tests::{
        COMMIT_ONE, EMPTY, TS, check_node, commit_waiver, lever, rule_waiver, signer, waiver,
    };

    const COMMIT_ZERO: &str = "sha1:0000000000000000000000000000000000000000";

    fn signed(node: Node) -> SignedLever {
        SignedLever {
            node,
            sig: "example-signature".into(),
        }
    }

    fn root() -> Root {
        Root {
            v: V1,
            commit: COMMIT_ONE.parse().unwrap(),
            tree: GitOid::EMPTY_TREE_SHA1.parse().unwrap(),
            ts: TS.parse().unwrap(),
            covers: Covers::List(vec![COMMIT_ONE.parse().unwrap()]),
            parents: vec![],
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
            counts: Counts::default(),
            signer: signer(),
        }
    }

    fn declared_off() -> Root {
        Root {
            state: State::DeclaredOff,
            evidence: "sha256:60d73f36e3a666adcfe47ee42cd116a0c910364dcb1f096477d9d1e56e9b7d40"
                .parse()
                .unwrap(),
            levers: vec![signed(commit_waiver())],
            counts: Counts {
                waivers: 1,
                ..Counts::default()
            },
            ..root()
        }
    }

    #[test]
    fn root_id_known_answer() {
        // DESIGN.md section 9.4: a declared_off root with one commit waiver. Changing these
        // bytes changes every root id ever written.
        let want = concat!(
            r#"{"commit":"sha1:0000000000000000000000000000000000000001","counts":{"checks":0,"fail":0,"findings":{"blocker":0,"major":0,"minor":0},"pass":0,"waivers":1},"#,
            r#""covers":["sha1:0000000000000000000000000000000000000001"],"evidence":"sha256:60d73f36e3a666adcfe47ee42cd116a0c910364dcb1f096477d9d1e56e9b7d40","#,
            r#""gate":{"binary":"sha256:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855","version":"0.1.0"},"#,
            r#""levers":[{"node":{"body":{"action":"waive","reason":"example reason","scope":{"commit":"sha1:0000000000000000000000000000000000000001"},"#,
            r#""signer":{"fingerprint":"SHA256:AAAA","principal":"dev@example.com"}},"kind":"lever","policy":"sha256:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855","#,
            r#""tree":"sha1:4b825dc642cb6eb9a060e54bf8d69288fbee4904","ts":"2026-10-10T18:30:00Z","v":1},"sig":"example-signature"}],"parents":[],"#,
            r#""policy":{"hash":"sha256:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855","source":"repo","version":1},"#,
            r#""signer":{"fingerprint":"SHA256:AAAA","principal":"dev@example.com"},"state":"declared_off","tree":"sha1:4b825dc642cb6eb9a060e54bf8d69288fbee4904","ts":"2026-10-10T18:30:00Z","v":1}"#
        );
        let r = declared_off();
        assert_eq!(
            String::from_utf8(r.canonical_bytes().unwrap()).unwrap(),
            want
        );
        assert_eq!(
            r.id().unwrap().to_string(),
            "sha256:01e449f228a88b336568a36424831f2c866c94d24b0916312442da7d89b67091"
        );
        let lever_id = r.levers[0].node.id().unwrap();
        assert_eq!(crate::merkle::merkle_root(&[lever_id]).unwrap(), r.evidence);
        assert_eq!(from_canonical::<Root>(want.as_bytes()).unwrap(), r);
        assert_eq!(r.validate(), Ok(()));
    }

    #[test]
    fn a_range_and_a_rule_waiver_round_trip() {
        let r = Root {
            covers: Covers::Range {
                base: COMMIT_ZERO.parse().unwrap(),
                tip: COMMIT_ONE.parse().unwrap(),
            },
            levers: vec![signed(rule_waiver())],
            counts: Counts {
                waivers: 1,
                ..Counts::default()
            },
            ..root()
        };
        let bytes = r.canonical_bytes().unwrap();
        assert_eq!(from_canonical::<Root>(&bytes).unwrap(), r);
        let json = String::from_utf8(bytes).unwrap();
        assert!(
            json.contains(r#""covers":{"base":"#)
                && json.contains(r#""expires":"2026-10-24T18:30:00Z""#),
            "{json}"
        );
        assert_eq!(r.validate(), Ok(()));
    }

    #[test]
    fn unknown_fields_and_versions_are_refused() {
        let good = serde_json::to_value(declared_off()).unwrap();
        let mut extra = good.clone();
        extra["note"] = "x".into();
        let mut v2 = good.clone();
        v2["v"] = 2.into();
        let mut bad_state = good.clone();
        bad_state["state"] = "absent".into();
        let mut extra_counts = good.clone();
        extra_counts["counts"]["errors"] = 0.into();
        let mut extra_lever = good.clone();
        extra_lever["levers"][0]["key"] = "x".into();
        let mut lever_v2 = good.clone();
        lever_v2["levers"][0]["node"]["v"] = 2.into();
        let mut default_source = good.clone();
        default_source["policy"]["source"] = "default".into();
        let mut range = serde_json::to_value(root()).unwrap();
        range["covers"] = serde_json::json!({"base": COMMIT_ZERO, "tip": COMMIT_ONE, "x": 1});
        for (name, doc) in [
            ("extra key", extra),
            ("v2", v2),
            ("state absent", bad_state),
            ("extra count", extra_counts),
            ("extra lever key", extra_lever),
            ("lever v2", lever_v2),
            ("policy source default", default_source),
            ("extra covers key", range),
        ] {
            assert!(
                serde_json::from_value::<Root>(doc).is_err(),
                "{name} was accepted"
            );
        }
    }

    #[test]
    fn validate_refuses_every_wrong_shape() {
        let other_commit = waiver(
            Scope::Commit {
                commit: COMMIT_ZERO.parse().unwrap(),
            },
            None,
        );
        let rule_no_expiry = waiver(
            Scope::Rule {
                rule: "JT-SUP-001".into(),
                path: "src/".into(),
            },
            None,
        );
        let with = |levers: Vec<Node>, state: State| Root {
            state,
            levers: levers.into_iter().map(signed).collect(),
            ..root()
        };
        let table = [
            ("covered, no levers", root(), Ok(())),
            (
                "rule waiver keeps covered",
                with(vec![rule_waiver()], State::Covered),
                Ok(()),
            ),
            (
                "check node as lever",
                with(vec![commit_waiver(), check_node()], State::Covered),
                Err(RootError::NotAWaiver { index: 1 }),
            ),
            (
                "init lever",
                with(vec![lever(Action::Init)], State::Covered),
                Err(RootError::NotAWaiver { index: 0 }),
            ),
            (
                "waiver without scope",
                with(vec![commit_waiver(), lever(Action::Waive)], State::Covered),
                Err(RootError::WaiverScope { index: 1 }),
            ),
            (
                "waiver for another commit",
                with(vec![other_commit], State::Covered),
                Err(RootError::WaiverCommit { index: 0 }),
            ),
            (
                "rule waiver without expiry",
                with(vec![rule_waiver(), rule_no_expiry], State::Covered),
                Err(RootError::WaiverExpiry { index: 1 }),
            ),
            (
                "declared_off with rule waiver only",
                with(vec![rule_waiver()], State::DeclaredOff),
                Err(RootError::DeclaredOffWithoutWaiver),
            ),
            (
                "declared_off with no levers",
                with(vec![], State::DeclaredOff),
                Err(RootError::DeclaredOffWithoutWaiver),
            ),
            ("declared_off with commit waiver", declared_off(), Ok(())),
            (
                "covers empty",
                Root {
                    covers: Covers::List(vec![]),
                    ..root()
                },
                Err(RootError::CoversTip),
            ),
            (
                "covers ends elsewhere",
                Root {
                    covers: Covers::List(vec![
                        COMMIT_ONE.parse().unwrap(),
                        COMMIT_ZERO.parse().unwrap(),
                    ]),
                    ..root()
                },
                Err(RootError::CoversTip),
            ),
            (
                "range tip elsewhere",
                Root {
                    covers: Covers::Range {
                        base: COMMIT_ZERO.parse().unwrap(),
                        tip: COMMIT_ZERO.parse().unwrap(),
                    },
                    ..root()
                },
                Err(RootError::CoversTip),
            ),
        ];
        for (name, r, want) in table {
            assert_eq!(r.validate(), want, "{name}");
        }
    }
}
