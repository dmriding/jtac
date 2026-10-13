//! The root: one signed summary per push tip (DESIGN.md section 9.4).

use serde::{Deserialize, Serialize};

use crate::canonical::{CanonicalError, canonical_bytes, canonical_digest};
use crate::digest::Sha256Digest;
use crate::node::{Body, Node, Scope};
use crate::oid::GitOid;
use crate::schema::V1;
use crate::signer::Signer;
use crate::time::Ts;
use crate::waiver::{OutOfScope, scope_at};

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
        self.validate_levers()?;
        // DESIGN.md 11.6: a commit waiver and declared_off imply each other.
        let commit_waived = self.levers.iter().any(SignedLever::covers_commit);
        match self.state {
            State::DeclaredOff if !commit_waived => {
                return Err(RootError::DeclaredOffWithoutWaiver);
            }
            State::Covered if commit_waived => return Err(RootError::CoveredWithCommitWaiver),
            _ => {}
        }
        if self.counts.waivers != self.levers.len() as u64 {
            return Err(RootError::CountsWaivers);
        }
        let tip = match &self.covers {
            Covers::List(list) => {
                if has_duplicate(list) {
                    return Err(RootError::CoversDuplicate);
                }
                list.last()
            }
            Covers::Range { base, tip } if base == tip => return Err(RootError::CoversRangeOfOne),
            Covers::Range { tip, .. } => Some(tip),
        };
        if tip != Some(&self.commit) {
            return Err(RootError::CoversTip);
        }
        if let Some(index) = self.parents.iter().position(|p| p.commit == self.commit) {
            return Err(RootError::ParentSelf { index });
        }
        if has_duplicate(&self.parents.iter().map(|p| p.commit).collect::<Vec<_>>()) {
            return Err(RootError::ParentDuplicate);
        }
        let covers: Vec<&GitOid> = match &self.covers {
            Covers::List(list) => list.iter().collect(),
            Covers::Range { base, tip } => vec![base, tip],
        };
        let parents = self.parents.iter().map(|p| &p.commit);
        let sha1 = |oid: &GitOid| matches!(oid, GitOid::Sha1(_));
        let commit_sha1 = sha1(&self.commit);
        if [&self.tree]
            .into_iter()
            .chain(covers)
            .chain(parents)
            .any(|o| sha1(o) != commit_sha1)
        {
            return Err(RootError::OidAlgorithm);
        }
        Ok(())
    }

    fn validate_levers(&self) -> Result<(), RootError> {
        for (index, lever) in self.levers.iter().enumerate() {
            if self.levers[..index].iter().any(|l| l.node == lever.node) {
                return Err(RootError::DuplicateWaiver { index });
            }
            scope_at(&lever.node, &self.commit, self.ts).map_err(|why| match why {
                OutOfScope::NotAWaiver => RootError::NotAWaiver { index },
                OutOfScope::NoScope => RootError::WaiverScope { index },
                OutOfScope::OtherCommit => RootError::WaiverCommit { index },
                OutOfScope::NoExpiry => RootError::WaiverExpiry { index },
                OutOfScope::Future => RootError::WaiverFuture { index },
                OutOfScope::Expired => RootError::WaiverExpired { index },
                OutOfScope::TooLong => RootError::WaiverTooLong { index },
                OutOfScope::OidAlgorithm => RootError::OidAlgorithm,
            })?;
        }
        Ok(())
    }
}

fn has_duplicate(oids: &[GitOid]) -> bool {
    (1..oids.len()).any(|i| oids[i..].contains(&oids[i - 1]))
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
    /// A waiver written after the root it claims to apply to.
    #[error("levers[{index}] was written after the root's ts")]
    WaiverFuture {
        /// Where.
        index: usize,
    },
    /// A waiver that had already expired at the root's `ts`.
    #[error("levers[{index}] expired before the root's ts")]
    WaiverExpired {
        /// Where.
        index: usize,
    },
    /// A waiver running past the 30-day cap from its own `ts`.
    #[error("levers[{index}] expires more than 30 days after it was written")]
    WaiverTooLong {
        /// Where.
        index: usize,
    },
    /// The same waiver listed twice.
    #[error("levers[{index}] repeats an earlier lever")]
    DuplicateWaiver {
        /// Where.
        index: usize,
    },
    /// `declared_off` needs a waiver that covers the whole commit.
    #[error("state is declared_off but no lever waives the whole commit")]
    DeclaredOffWithoutWaiver,
    /// A commit waiver turns the gate off; the root cannot claim `covered`.
    #[error("state is covered but a lever waives the whole commit")]
    CoveredWithCommitWaiver,
    /// `counts.waivers` is not the length of `levers`.
    #[error("counts.waivers is not the number of levers")]
    CountsWaivers,
    /// `covers` lists a commit twice.
    #[error("covers lists a commit twice")]
    CoversDuplicate,
    /// A range of one commit; the list form is its one spelling.
    #[error("covers is a range whose base is its tip")]
    CoversRangeOfOne,
    /// `covers` does not end at the root's commit.
    #[error("covers does not end at the root's commit")]
    CoversTip,
    /// A parent link to this root's own commit.
    #[error("parents[{index}] is the root's own commit")]
    ParentSelf {
        /// Where.
        index: usize,
    },
    /// Two parent links to one commit.
    #[error("parents lists a commit twice")]
    ParentDuplicate,
    /// A git id whose hash algorithm differs from `commit`'s.
    #[error("every git id in a root must use the commit's hash algorithm")]
    OidAlgorithm,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::canonical::from_canonical;
    use crate::node::Action;
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

    fn with(levers: Vec<Node>, state: State) -> Root {
        Root {
            state,
            counts: Counts {
                waivers: levers.len() as u64,
                ..Counts::default()
            },
            levers: levers.into_iter().map(signed).collect(),
            ..root()
        }
    }

    fn rule(expires: Option<&str>) -> Node {
        waiver(
            Scope::Rule {
                rule: "JT-SUP-001".into(),
                path: "src/".into(),
            },
            expires,
        )
    }

    fn rule_written(ts: &str, expires: &str) -> Node {
        Node {
            ts: ts.parse().unwrap(),
            ..rule(Some(expires))
        }
    }

    fn with_expiry(mut node: Node, expires: &str) -> Node {
        if let Body::Lever(b) = &mut node.body {
            b.expires = Some(expires.parse().unwrap());
        }
        node
    }

    fn parent(commit: GitOid) -> Parent {
        Parent {
            commit,
            root: EMPTY.parse().unwrap(),
        }
    }

    fn check(table: Vec<(&str, Root, Result<(), RootError>)>) {
        for (name, r, want) in table {
            assert_eq!(r.validate(), want, "{name}");
        }
    }

    #[test]
    fn validate_refuses_wrong_waivers() {
        let other_commit = waiver(
            Scope::Commit {
                commit: COMMIT_ZERO.parse().unwrap(),
            },
            None,
        );
        let later = rule(Some("2026-10-20T18:30:00Z"));
        check(vec![
            ("covered, no levers", root(), Ok(())),
            (
                "rule waiver keeps covered",
                with(vec![rule_waiver()], State::Covered),
                Ok(()),
            ),
            ("declared_off with commit waiver", declared_off(), Ok(())),
            (
                "check node as lever",
                with(vec![commit_waiver(), check_node()], State::DeclaredOff),
                Err(RootError::NotAWaiver { index: 1 }),
            ),
            (
                "init lever",
                with(vec![lever(Action::Init)], State::Covered),
                Err(RootError::NotAWaiver { index: 0 }),
            ),
            (
                "same waiver twice",
                with(vec![rule_waiver(), rule_waiver()], State::Covered),
                Err(RootError::DuplicateWaiver { index: 1 }),
            ),
            (
                "non-adjacent duplicate",
                with(vec![rule_waiver(), later, rule_waiver()], State::Covered),
                Err(RootError::DuplicateWaiver { index: 2 }),
            ),
            (
                "waiver without scope",
                with(
                    vec![commit_waiver(), lever(Action::Waive)],
                    State::DeclaredOff,
                ),
                Err(RootError::WaiverScope { index: 1 }),
            ),
            (
                "waiver for another commit",
                with(vec![other_commit], State::DeclaredOff),
                Err(RootError::WaiverCommit { index: 0 }),
            ),
            (
                "rule waiver without expiry",
                with(vec![rule_waiver(), rule(None)], State::Covered),
                Err(RootError::WaiverExpiry { index: 1 }),
            ),
            (
                "covered with a commit waiver",
                with(vec![commit_waiver()], State::Covered),
                Err(RootError::CoveredWithCommitWaiver),
            ),
            (
                "covered with rule and commit waivers",
                with(vec![rule_waiver(), commit_waiver()], State::Covered),
                Err(RootError::CoveredWithCommitWaiver),
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
            (
                "counts.waivers too low",
                Root {
                    counts: Counts::default(),
                    ..with(vec![rule_waiver()], State::Covered)
                },
                Err(RootError::CountsWaivers),
            ),
        ]);
    }

    #[test]
    fn validate_bounds_waiver_time_by_root_ts_and_write_ts() {
        check(vec![
            (
                "expired waiver",
                with(vec![rule(Some("2020-01-01T00:00:00Z"))], State::Covered),
                Err(RootError::WaiverExpired { index: 0 }),
            ),
            (
                "waiver expired one second before ts",
                with(vec![rule(Some("2026-10-10T18:29:59Z"))], State::Covered),
                Err(RootError::WaiverExpired { index: 0 }),
            ),
            (
                "commit waiver expired one second before ts",
                with(
                    vec![with_expiry(commit_waiver(), "2026-10-10T18:29:59Z")],
                    State::DeclaredOff,
                ),
                Err(RootError::WaiverExpired { index: 0 }),
            ),
            (
                "waiver expiring at ts",
                with(vec![rule(Some(TS))], State::Covered),
                Ok(()),
            ),
            (
                "waiver expiring at the cap",
                with(vec![rule(Some("2026-11-09T18:30:00Z"))], State::Covered),
                Ok(()),
            ),
            (
                "waiver past the cap",
                with(
                    vec![rule_waiver(), rule(Some("2026-11-09T18:30:01Z"))],
                    State::Covered,
                ),
                Err(RootError::WaiverTooLong { index: 1 }),
            ),
            (
                "older waiver within its own cap",
                with(
                    vec![rule_written("2026-09-20T18:30:00Z", "2026-10-20T18:30:00Z")],
                    State::Covered,
                ),
                Ok(()),
            ),
            (
                "older waiver past its cap but near the root",
                with(
                    vec![rule_written("2026-09-20T18:30:00Z", "2026-10-25T18:30:00Z")],
                    State::Covered,
                ),
                Err(RootError::WaiverTooLong { index: 0 }),
            ),
            (
                "waiver written after the root",
                with(
                    vec![rule_written("2026-10-10T18:30:01Z", "2026-10-20T18:30:00Z")],
                    State::Covered,
                ),
                Err(RootError::WaiverFuture { index: 0 }),
            ),
            (
                "commit waiver with an expiry in range",
                with(
                    vec![with_expiry(commit_waiver(), "2026-10-20T18:30:00Z")],
                    State::DeclaredOff,
                ),
                Ok(()),
            ),
            (
                "commit waiver already expired",
                with(
                    vec![with_expiry(commit_waiver(), "2020-01-01T00:00:00Z")],
                    State::DeclaredOff,
                ),
                Err(RootError::WaiverExpired { index: 0 }),
            ),
            (
                "commit waiver past the cap",
                with(
                    vec![with_expiry(commit_waiver(), "2026-11-09T18:30:01Z")],
                    State::DeclaredOff,
                ),
                Err(RootError::WaiverTooLong { index: 0 }),
            ),
        ]);
    }

    #[test]
    fn validate_refuses_wrong_covers_counts_and_parents() {
        let one: GitOid = COMMIT_ONE.parse().unwrap();
        let zero: GitOid = COMMIT_ZERO.parse().unwrap();
        check(vec![
            (
                "counts.waivers too high",
                Root {
                    counts: Counts {
                        waivers: 5,
                        ..Counts::default()
                    },
                    ..root()
                },
                Err(RootError::CountsWaivers),
            ),
            (
                "covers lists a commit twice",
                Root {
                    covers: Covers::List(vec![one, zero, one]),
                    ..root()
                },
                Err(RootError::CoversDuplicate),
            ),
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
                    covers: Covers::List(vec![one, zero]),
                    ..root()
                },
                Err(RootError::CoversTip),
            ),
            (
                "range tip elsewhere",
                Root {
                    covers: Covers::Range {
                        base: one,
                        tip: zero,
                    },
                    ..root()
                },
                Err(RootError::CoversTip),
            ),
            (
                "range of one commit",
                Root {
                    covers: Covers::Range {
                        base: one,
                        tip: one,
                    },
                    ..root()
                },
                Err(RootError::CoversRangeOfOne),
            ),
            (
                "parent is self",
                Root {
                    parents: vec![parent(one)],
                    ..root()
                },
                Err(RootError::ParentSelf { index: 0 }),
            ),
            (
                "self parent at index 1",
                Root {
                    parents: vec![parent(zero), parent(one)],
                    ..root()
                },
                Err(RootError::ParentSelf { index: 1 }),
            ),
            (
                "parent listed twice",
                Root {
                    parents: vec![parent(zero), parent(zero)],
                    ..root()
                },
                Err(RootError::ParentDuplicate),
            ),
        ]);
    }

    #[test]
    fn validate_refuses_mixed_hash_algorithms() {
        let one: GitOid = COMMIT_ONE.parse().unwrap();
        let zero: GitOid = COMMIT_ZERO.parse().unwrap();
        let later = rule(Some("2026-10-20T18:30:00Z"));
        let s256: GitOid =
            "sha256:6ef19b41225c5369f1c104d45d8d85efa9b057b53b14b4b9b939dd74decc5321"
                .parse()
                .unwrap();
        check(vec![
            (
                "sha1 commit with sha256 tree",
                Root {
                    tree: s256,
                    ..root()
                },
                Err(RootError::OidAlgorithm),
            ),
            (
                "sha256 commit with sha1 tree",
                Root {
                    commit: s256,
                    covers: Covers::List(vec![s256]),
                    ..root()
                },
                Err(RootError::OidAlgorithm),
            ),
            (
                "sha256 commit in covers",
                Root {
                    covers: Covers::List(vec![s256, one]),
                    ..root()
                },
                Err(RootError::OidAlgorithm),
            ),
            (
                "sha256 base in a range",
                Root {
                    covers: Covers::Range {
                        base: s256,
                        tip: one,
                    },
                    ..root()
                },
                Err(RootError::OidAlgorithm),
            ),
            (
                "sha256 parent at index 1",
                Root {
                    parents: vec![parent(zero), parent(s256)],
                    ..root()
                },
                Err(RootError::OidAlgorithm),
            ),
            (
                "sha256 lever tree at index 1",
                with(
                    vec![
                        rule_waiver(),
                        Node {
                            tree: s256,
                            ..later
                        },
                    ],
                    State::Covered,
                ),
                Err(RootError::OidAlgorithm),
            ),
        ]);
    }
}
