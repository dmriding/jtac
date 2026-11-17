//! The root: one signed summary per push tip (DESIGN.md section 9.4).

use serde::{Deserialize, Serialize};

use crate::canonical::{CanonicalError, canonical_bytes, canonical_digest};
use crate::digest::Sha256Digest;
use crate::from::FromHead;
use crate::node::{Body, Node, Scope};
use crate::oid::GitOid;
use crate::schema::RootVersion;
use crate::signer::Signer;
use crate::time::Ts;
use crate::waiver::{OutOfScope, Waivable, scope_at};

/// What a push tip's evidence adds up to. Signed as a whole; applied waivers are the only
/// nodes that travel with it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Root {
    /// Schema version: 1, 2 with `base` and `from` (DESIGN.md section 15), or 3 with
    /// `observed`, `unread` and the state `failed` (section 15.2).
    pub v: RootVersion,
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
    /// Version 2: the commit the rules diffed from; absent on a parentless tip.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base: Option<GitOid>,
    /// Version 2, a merge root alone: the head it stands for.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from: Option<FromHead>,
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
    /// Version 3, a root with `from` alone: each check read from CI, its node in full.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub observed: Vec<Node>,
    /// Version 3, a root without `from` alone: the `ci` checks its gate left to CI.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub unread: Vec<String>,
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

    /// A root as `refs/jtac/roots/` may hold one: its shape, and never the state `failed`,
    /// so a failure record placed there passes for no root (DESIGN.md section 15.2).
    pub fn validate(&self) -> Result<(), RootError> {
        self.shape()?;
        match self.state {
            State::Failed => Err(RootError::FailedRoot),
            _ => Ok(()),
        }
    }

    /// A failure record as `refs/jtac/failed/` may hold one: a root's shape and the state
    /// `failed` alone, so a root placed there passes for no record.
    pub fn validate_record(&self) -> Result<(), RootError> {
        self.shape()?;
        match self.state {
            State::Failed => Ok(()),
            _ => Err(RootError::NotARecord),
        }
    }

    /// Shape rules a reader checks before trusting any field, a root's and a failure
    /// record's alike. Signature, Merkle root, and counts are checked by their own code;
    /// this is only what the type cannot express.
    pub fn shape(&self) -> Result<(), RootError> {
        self.validate_version()?;
        self.validate_observed()?;
        self.validate_levers()?;
        // DESIGN.md 11.6: a commit waiver and declared_off imply each other.
        let commit_waived = self.levers.iter().any(SignedLever::covers_commit);
        match self.state {
            State::DeclaredOff if !commit_waived => {
                return Err(RootError::DeclaredOffWithoutWaiver);
            }
            State::Covered | State::Failed if commit_waived => {
                return Err(RootError::CoveredWithCommitWaiver);
            }
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
            .chain(self.base.as_ref())
            .chain(self.from.as_ref().map(|f| &f.head))
            .any(|o| sha1(o) != commit_sha1)
        {
            return Err(RootError::OidAlgorithm);
        }
        Ok(())
    }

    /// Version 2's shape (DESIGN.md section 15, decision 46): `base` is below the tip and
    /// `covers` reaches nothing at or below it, since a root vouches for nothing below the
    /// diff it judged; no `base` is a parentless tip, covered alone; a root with `from` names
    /// a head other than its tip, a `compared` exactly when it names a root, `base`'s root as
    /// its only parent, and a bounded `covers` that begins at `base`. Version 1 names neither.
    fn validate_version(&self) -> Result<(), RootError> {
        if self.v == RootVersion::V1 {
            return match (&self.base, &self.from) {
                (None, None) => Ok(()),
                (Some(_), _) => Err(RootError::VersionField {
                    v: 1,
                    field: "base",
                }),
                (None, Some(_)) => Err(RootError::VersionField {
                    v: 1,
                    field: "from",
                }),
            };
        }
        match &self.base {
            Some(base) if *base == self.commit => return Err(RootError::BaseIsTip),
            Some(base) => {
                let (listed, lower) = match &self.covers {
                    Covers::List(list) => (list.contains(base), None),
                    Covers::Range { base: lower, .. } => (false, Some(lower)),
                };
                if listed || (lower == Some(base) && self.from.is_none()) {
                    return Err(RootError::CoversBase);
                }
                if self.from.is_some() && lower.is_some_and(|lower| lower != base) {
                    return Err(RootError::FromRangeBase);
                }
            }
            None if self.from.is_some() => return Err(RootError::FromWithoutBase),
            None => {
                if self.covers != Covers::List(vec![self.commit]) || !self.parents.is_empty() {
                    return Err(RootError::NoBaseHistory);
                }
            }
        }
        if let Some(from) = &self.from {
            if from.head == self.commit {
                return Err(RootError::FromSelf);
            }
            if from.root.is_some() != from.compared.is_some() {
                return Err(RootError::FromCompared);
            }
            if self.parents.len() > 1 || self.parents.iter().any(|p| Some(p.commit) != self.base) {
                return Err(RootError::FromParents);
            }
        }
        Ok(())
    }

    /// Version 3's shape, as far as a root alone shows it (DESIGN.md section 15.2): `unread`
    /// on a root without `from`, a check read from CI on one with it, each for this root's
    /// commit, tree and policy; `failed` on a failure record, which names `from`; and a
    /// version 3 document with none of the three is no version 3 document. Whether `unread`
    /// is the policy's `ci` checks is the reader's to ask of the tree (decision 57).
    fn validate_observed(&self) -> Result<(), RootError> {
        let v = match self.v {
            RootVersion::V1 => 1,
            RootVersion::V2 => 2,
            RootVersion::V3 => 3,
        };
        let failed = self.state == State::Failed;
        if v < 3 {
            let field = match () {
                () if !self.observed.is_empty() => "observed",
                () if !self.unread.is_empty() => "unread",
                () if failed => return Err(RootError::FailedShape),
                () => return Ok(()),
            };
            return Err(RootError::VersionField { v, field });
        }
        match &self.from {
            Some(_) if !self.unread.is_empty() => return Err(RootError::UnreadWithFrom),
            None if !self.observed.is_empty() => return Err(RootError::ObservedWithoutFrom),
            None if failed => return Err(RootError::FailedShape),
            _ => {}
        }
        if self.observed.is_empty() && self.unread.is_empty() && !failed {
            return Err(RootError::VersionThreeEmpty);
        }
        let mut ids = Vec::new();
        for (index, node) in self.observed.iter().enumerate() {
            let Body::Observed(check) = &node.body else {
                return Err(RootError::ObservedNode { index });
            };
            let ours = node.tree == self.tree
                && node.policy == self.policy.hash
                && check.observed.commit == self.commit;
            if !ours || !check.coherent() {
                return Err(RootError::ObservedNode { index });
            }
            if ids.contains(&&check.id) {
                return Err(RootError::ObservedDuplicate { index });
            }
            ids.push(&check.id);
        }
        if (1..self.unread.len()).any(|i| self.unread[i..].contains(&self.unread[i - 1])) {
            return Err(RootError::UnreadDuplicate);
        }
        Ok(())
    }

    fn validate_levers(&self) -> Result<(), RootError> {
        // A from root's commit waivers name the pull request's commits, which the CI
        // signature attests and a reader's clone may not hold (DESIGN.md section 15).
        let waivable = match self.from {
            Some(_) => Waivable::Any,
            None => Waivable::Tip,
        };
        for (index, lever) in self.levers.iter().enumerate() {
            if self.levers[..index].iter().any(|l| l.node == lever.node) {
                return Err(RootError::DuplicateWaiver { index });
            }
            scope_at(&lever.node, &self.commit, waivable, self.ts).map_err(|why| match why {
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
        matches!(&self.node.body, Body::Lever(b) if matches!(b.scope, Some(Scope::Commit { gate: None, .. })))
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

/// Coverage state a root can assert, and `failed`, which a failure record alone holds.
/// Absent is the missing root, never a value.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum State {
    /// The gate ran and passed.
    Covered,
    /// A waiver covers the whole commit.
    DeclaredOff,
    /// The merge job's verdict failed: a failure record's, never a root's (section 15.2).
    Failed,
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
    /// A commit waiver turns the gate off; the root cannot claim `covered`, nor a failure.
    #[error("state is not declared_off but a lever waives the whole commit")]
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
    /// A root with a later version's field.
    #[error("a version {v} root has no `{field}` field")]
    VersionField {
        /// The root's version, 1 or 2.
        v: u8,
        /// `base`, `from`, `observed` or `unread`.
        field: &'static str,
    },
    /// The state `failed` under `refs/jtac/roots/`.
    #[error("the state `failed` is a failure record's, under refs/jtac/failed/, never a root's")]
    FailedRoot,
    /// Any other state under `refs/jtac/failed/`.
    #[error(
        "a failure record holds the state `failed`; this is a root, which refs/jtac/roots/ holds"
    )]
    NotARecord,
    /// `failed` on anything but a version 3 document with `from`.
    #[error("the state `failed` is a version 3 failure record's, which names `from`")]
    FailedShape,
    /// `unread` beside `from`: the merge job reads every check.
    #[error("a root with `from` lists no `unread`; the merge job reads every check")]
    UnreadWithFrom,
    /// A check read from CI in a root the local gate wrote.
    #[error("a root without `from` holds no check read from CI")]
    ObservedWithoutFrom,
    /// An `observed` entry that is not a coherent version 3 check node for this root's
    /// commit, tree and policy.
    #[error(
        "observed[{index}] is not a check read from CI for this root's commit, tree and policy"
    )]
    ObservedNode {
        /// Where.
        index: usize,
    },
    /// Two `observed` entries for one check.
    #[error("observed[{index}] repeats an earlier check id")]
    ObservedDuplicate {
        /// Where.
        index: usize,
    },
    /// `unread` lists a check twice.
    #[error("unread lists a check twice")]
    UnreadDuplicate,
    /// A version 3 document that needs no version 3.
    #[error(
        "a version 3 root holds a check read from CI, lists one unread, or is a failure record"
    )]
    VersionThreeEmpty,
    /// `base` is the root's own commit.
    #[error("base is the root's own commit")]
    BaseIsTip,
    /// `covers` names `base`, or begins at it on a root without `from`.
    #[error("covers reaches base; a root vouches for nothing at or below the diff it judged")]
    CoversBase,
    /// A merge root's bounded `covers` must begin at `base`.
    #[error("a from root's covers range does not begin at its base")]
    FromRangeBase,
    /// A merge root with no `base`.
    #[error("a from root names no base")]
    FromWithoutBase,
    /// No `base` means a parentless tip: `covers` is the tip alone and `parents` is empty.
    #[error("a root with no base names a parentless tip; covers is the tip alone, parents empty")]
    NoBaseHistory,
    /// `from.head` is the root's own commit.
    #[error("from names the root's own commit as the head")]
    FromSelf,
    /// `from.root` and `from.compared` come together.
    #[error("from names a root without compared, or compared without a root")]
    FromCompared,
    /// A merge root's parents is `base`'s root alone.
    #[error("a from root's parents is base's root alone")]
    FromParents,
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::canonical::from_canonical;
    use crate::compare::Compared;
    use crate::node::tests::{
        COMMIT_ONE, EMPTY, TS, check_node, ci_waiver, commit_waiver, lever, observed_node,
        rule_waiver, signer, waiver,
    };
    use crate::node::{Action, Gate};

    const COMMIT_ZERO: &str = "sha1:0000000000000000000000000000000000000000";

    fn signed(node: Node) -> SignedLever {
        SignedLever {
            node,
            sig: "example-signature".into(),
        }
    }

    fn root() -> Root {
        Root {
            v: RootVersion::V1,
            commit: COMMIT_ONE.parse().unwrap(),
            tree: GitOid::EMPTY_TREE_SHA1.parse().unwrap(),
            ts: TS.parse().unwrap(),
            covers: Covers::List(vec![COMMIT_ONE.parse().unwrap()]),
            parents: vec![],
            base: None,
            from: None,
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
            observed: vec![],
            unread: vec![],
            counts: Counts::default(),
            signer: signer(),
        }
    }

    /// A version 2 root on `COMMIT_ONE` whose rules diffed from `COMMIT_ZERO`.
    pub(crate) fn v2() -> Root {
        Root {
            v: RootVersion::V2,
            base: Some(COMMIT_ZERO.parse().unwrap()),
            ..root()
        }
    }

    fn from_head() -> FromHead {
        FromHead {
            head: "sha1:0000000000000000000000000000000000000002"
                .parse()
                .unwrap(),
            root: Some(EMPTY.parse().unwrap()),
            compared: Some(Compared::Agreed),
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
        let mut v4 = good.clone();
        v4["v"] = 4.into();
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
            ("v4", v4),
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
                gate: None,
            },
            None,
        );
        let ci_elsewhere = waiver(
            Scope::Commit {
                commit: COMMIT_ZERO.parse().unwrap(),
                gate: Some(Gate::Ci),
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
                "ci waiver keeps covered",
                with(vec![ci_waiver()], State::Covered),
                Ok(()),
            ),
            (
                "ci waiver alone does not declare off",
                with(vec![ci_waiver()], State::DeclaredOff),
                Err(RootError::DeclaredOffWithoutWaiver),
            ),
            (
                "ci waiver for another commit",
                with(vec![ci_elsewhere], State::Covered),
                Err(RootError::WaiverCommit { index: 0 }),
            ),
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
    fn validate_accepts_version_two_shapes() {
        let one: GitOid = COMMIT_ONE.parse().unwrap();
        let zero: GitOid = COMMIT_ZERO.parse().unwrap();
        let two: GitOid = "sha1:0000000000000000000000000000000000000002"
            .parse()
            .unwrap();
        let merge = || Root {
            from: Some(from_head()),
            parents: vec![parent(zero)],
            ..v2()
        };
        check(vec![
            ("v2 with a base", v2(), Ok(())),
            (
                "v2 range beginning above base",
                Root {
                    covers: Covers::Range {
                        base: two,
                        tip: one,
                    },
                    ..v2()
                },
                Ok(()),
            ),
            (
                "v2 parentless tip",
                Root {
                    v: RootVersion::V2,
                    ..root()
                },
                Ok(()),
            ),
            ("from root", merge(), Ok(())),
            (
                "from root with no head root",
                Root {
                    from: Some(FromHead {
                        root: None,
                        compared: None,
                        ..from_head()
                    }),
                    ..merge()
                },
                Ok(()),
            ),
            (
                "from root with no parent",
                Root {
                    parents: vec![],
                    ..merge()
                },
                Ok(()),
            ),
            (
                "from root range beginning at base",
                Root {
                    covers: Covers::Range {
                        base: zero,
                        tip: one,
                    },
                    ..merge()
                },
                Ok(()),
            ),
        ]);
    }

    #[test]
    fn validate_refuses_wrong_version_two_shapes() {
        let one: GitOid = COMMIT_ONE.parse().unwrap();
        let zero: GitOid = COMMIT_ZERO.parse().unwrap();
        let two: GitOid = "sha1:0000000000000000000000000000000000000002"
            .parse()
            .unwrap();
        let merge = || Root {
            from: Some(from_head()),
            parents: vec![parent(zero)],
            ..v2()
        };
        check(vec![
            (
                "v1 with a base",
                Root {
                    base: Some(zero),
                    ..root()
                },
                Err(RootError::VersionField {
                    v: 1,
                    field: "base",
                }),
            ),
            (
                "v1 with a from",
                Root {
                    from: Some(from_head()),
                    ..root()
                },
                Err(RootError::VersionField {
                    v: 1,
                    field: "from",
                }),
            ),
            (
                "base is the tip",
                Root {
                    base: Some(one),
                    ..v2()
                },
                Err(RootError::BaseIsTip),
            ),
            (
                "covers lists base",
                Root {
                    covers: Covers::List(vec![zero, one]),
                    ..v2()
                },
                Err(RootError::CoversBase),
            ),
            (
                "covers range begins at base without from",
                Root {
                    covers: Covers::Range {
                        base: zero,
                        tip: one,
                    },
                    ..v2()
                },
                Err(RootError::CoversBase),
            ),
            (
                "from root range begins above base",
                Root {
                    covers: Covers::Range {
                        base: two,
                        tip: one,
                    },
                    ..merge()
                },
                Err(RootError::FromRangeBase),
            ),
            (
                "from root without base",
                Root {
                    base: None,
                    ..merge()
                },
                Err(RootError::FromWithoutBase),
            ),
            (
                "no base, covers beyond the tip",
                Root {
                    v: RootVersion::V2,
                    covers: Covers::List(vec![zero, one]),
                    ..root()
                },
                Err(RootError::NoBaseHistory),
            ),
            (
                "no base, a parent",
                Root {
                    v: RootVersion::V2,
                    parents: vec![parent(zero)],
                    ..root()
                },
                Err(RootError::NoBaseHistory),
            ),
            (
                "from names the tip",
                Root {
                    from: Some(FromHead {
                        head: one,
                        ..from_head()
                    }),
                    ..merge()
                },
                Err(RootError::FromSelf),
            ),
            (
                "from root without compared",
                Root {
                    from: Some(FromHead {
                        compared: None,
                        ..from_head()
                    }),
                    ..merge()
                },
                Err(RootError::FromCompared),
            ),
            (
                "from parent is not base",
                Root {
                    parents: vec![parent(two)],
                    ..merge()
                },
                Err(RootError::FromParents),
            ),
            (
                "from with two parents",
                Root {
                    parents: vec![parent(zero), parent(two)],
                    ..merge()
                },
                Err(RootError::FromParents),
            ),
        ]);
    }

    #[test]
    fn a_from_root_may_waive_any_commit_and_another_root_may_not() {
        let other = waiver(
            Scope::Commit {
                commit: "sha1:0000000000000000000000000000000000000007"
                    .parse()
                    .unwrap(),
                gate: None,
            },
            None,
        );
        let off = |r: Root| Root {
            state: State::DeclaredOff,
            levers: vec![signed(other.clone())],
            counts: Counts {
                waivers: 1,
                ..Counts::default()
            },
            ..r
        };
        let merge = Root {
            from: Some(from_head()),
            parents: vec![parent(COMMIT_ZERO.parse().unwrap())],
            ..v2()
        };
        assert_eq!(off(merge).validate(), Ok(()));
        assert_eq!(
            off(v2()).validate(),
            Err(RootError::WaiverCommit { index: 0 })
        );
        assert_eq!(
            off(root()).validate(),
            Err(RootError::WaiverCommit { index: 0 })
        );
    }

    #[test]
    fn version_two_round_trips_and_version_one_bytes_are_unchanged() {
        let merge = Root {
            from: Some(from_head()),
            parents: vec![parent(COMMIT_ZERO.parse().unwrap())],
            ..v2()
        };
        for r in [v2(), merge] {
            let bytes = r.canonical_bytes().unwrap();
            assert_eq!(from_canonical::<Root>(&bytes).unwrap(), r);
            assert!(String::from_utf8_lossy(&bytes).contains(r#""base":"sha1:"#));
        }
        let json = String::from_utf8(root().canonical_bytes().unwrap()).unwrap();
        assert!(!json.contains("base") && !json.contains("from"), "{json}");
    }

    #[test]
    fn root_v2_id_known_answer() {
        // DESIGN.md section 9.4: a covered version 2 root over the empty tree whose rules
        // diffed from the zero commit. Changing these bytes changes every version 2 root id.
        let r = v2();
        let json = String::from_utf8(r.canonical_bytes().unwrap()).unwrap();
        let want = concat!(
            r#"{"base":"sha1:0000000000000000000000000000000000000000""#,
            r#","commit":"sha1:0000000000000000000000000000000000000001","counts":{"checks":0,"fail":0"#,
            r#","findings":{"blocker":0,"major":0,"minor":0},"pass":0,"waivers":0}"#,
            r#","covers":["sha1:0000000000000000000000000000000000000001"]"#,
            r#","evidence":"sha256:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855""#,
            r#","gate":{"binary":"sha256:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855","version":"0.1.0"}"#,
            r#","levers":[],"parents":[]"#,
            r#","policy":{"hash":"sha256:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855","source":"repo""#,
            r#","version":1},"signer":{"fingerprint":"SHA256:AAAA","principal":"dev@example.com"},"state":"covered""#,
            r#","tree":"sha1:4b825dc642cb6eb9a060e54bf8d69288fbee4904","ts":"2026-10-10T18:30:00Z","v":2}"#
        );
        assert_eq!(json, want);
        assert_eq!(
            r.id().unwrap().to_string(),
            "sha256:d82da976599cef7f551d5ce1e0d8b25c498f70d0ce505f1dacaf50aecce48f49"
        );
        assert_eq!(from_canonical::<Root>(json.as_bytes()).unwrap(), r);
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

    fn v3_local() -> Root {
        Root {
            v: RootVersion::V3,
            unread: vec!["test".into()],
            ..v2()
        }
    }

    fn v3_merge() -> Root {
        Root {
            v: RootVersion::V3,
            from: Some(from_head()),
            parents: vec![parent(COMMIT_ZERO.parse().unwrap())],
            observed: vec![observed_node()],
            ..v2()
        }
    }

    #[test]
    fn version_three_is_unread_without_from_or_observed_with_it_or_a_failure_record() {
        let failed = || Root {
            state: State::Failed,
            ..v3_merge()
        };
        check(vec![
            ("the local gate's root lists unread", v3_local(), Ok(())),
            ("the merge root holds the check read", v3_merge(), Ok(())),
            (
                "a failure record under refs/jtac/roots/",
                failed(),
                Err(RootError::FailedRoot),
            ),
            (
                "unread beside from",
                Root {
                    unread: vec!["test".into()],
                    ..v3_merge()
                },
                Err(RootError::UnreadWithFrom),
            ),
            (
                "a check read from CI without from",
                Root {
                    observed: vec![observed_node()],
                    ..v3_local()
                },
                Err(RootError::ObservedWithoutFrom),
            ),
            (
                "failed without from",
                Root {
                    state: State::Failed,
                    ..v3_local()
                },
                Err(RootError::FailedShape),
            ),
            (
                "failed in version 2",
                Root {
                    v: RootVersion::V2,
                    observed: vec![],
                    ..failed()
                },
                Err(RootError::FailedShape),
            ),
            (
                "version 3 with nothing version 3 holds",
                Root {
                    unread: vec![],
                    ..v3_local()
                },
                Err(RootError::VersionThreeEmpty),
            ),
            (
                "a failure that a commit waiver would have declared off",
                Root {
                    levers: vec![signed(commit_waiver())],
                    counts: Counts {
                        waivers: 1,
                        ..Counts::default()
                    },
                    ..failed()
                },
                Err(RootError::CoveredWithCommitWaiver),
            ),
            (
                "unread in version 2",
                Root {
                    v: RootVersion::V2,
                    ..v3_local()
                },
                Err(RootError::VersionField {
                    v: 2,
                    field: "unread",
                }),
            ),
            (
                "observed in version 2",
                Root {
                    v: RootVersion::V2,
                    ..v3_merge()
                },
                Err(RootError::VersionField {
                    v: 2,
                    field: "observed",
                }),
            ),
        ]);
        assert_eq!(failed().shape(), Ok(()), "a failure record's own shape");
        assert_eq!(
            failed().validate_record(),
            Ok(()),
            "a failure record's own ref"
        );
        assert_eq!(v3_merge().validate_record(), Err(RootError::NotARecord));
        let waived = Root {
            unread: vec![],
            ..v3_local()
        };
        assert_eq!(
            Root {
                state: State::Failed,
                ..waived
            }
            .validate_record(),
            Err(RootError::FailedShape),
            "a record's shape is checked before its state"
        );
        assert_eq!(
            Root {
                observed: vec![],
                ..failed()
            }
            .shape(),
            Ok(()),
            "a failure record under a policy with no ci check"
        );
    }

    #[test]
    fn a_version_three_root_holds_each_check_once_for_its_own_commit_tree_and_policy() {
        let observed = |edit: fn(&mut Node)| {
            let mut node = observed_node();
            edit(&mut node);
            Root {
                observed: vec![node],
                ..v3_merge()
            }
        };
        check(vec![
            (
                "a run's node in observed",
                Root {
                    observed: vec![check_node()],
                    ..v3_merge()
                },
                Err(RootError::ObservedNode { index: 0 }),
            ),
            (
                "a check read for another commit",
                observed(|n| {
                    if let Body::Observed(c) = &mut n.body {
                        c.observed.commit = COMMIT_ZERO.parse().unwrap();
                    }
                }),
                Err(RootError::ObservedNode { index: 0 }),
            ),
            (
                "a check read under another policy",
                observed(|n| n.policy = Sha256Digest::of(b"other")),
                Err(RootError::ObservedNode { index: 0 }),
            ),
            (
                "a pass over the forge's failure",
                observed(|n| {
                    if let Body::Observed(c) = &mut n.body {
                        c.observed.conclusion = "failure".into();
                    }
                }),
                Err(RootError::ObservedNode { index: 0 }),
            ),
            (
                "one check read twice",
                Root {
                    observed: vec![observed_node(), observed_node()],
                    ..v3_merge()
                },
                Err(RootError::ObservedDuplicate { index: 1 }),
            ),
            (
                "one check unread twice",
                Root {
                    unread: vec!["test".into(), "e2e".into(), "test".into()],
                    ..v3_local()
                },
                Err(RootError::UnreadDuplicate),
            ),
        ]);
    }

    #[test]
    fn root_v3_id_known_answer() {
        // DESIGN.md section 9.4: the version 2 known answer, written by a local gate that
        // left `test` to CI. Changing these bytes changes every version 3 root id.
        let r = v3_local();
        let json = String::from_utf8(r.canonical_bytes().unwrap()).unwrap();
        let want = concat!(
            r#"{"base":"sha1:0000000000000000000000000000000000000000""#,
            r#","commit":"sha1:0000000000000000000000000000000000000001","counts":{"checks":0,"fail":0"#,
            r#","findings":{"blocker":0,"major":0,"minor":0},"pass":0,"waivers":0}"#,
            r#","covers":["sha1:0000000000000000000000000000000000000001"]"#,
            r#","evidence":"sha256:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855""#,
            r#","gate":{"binary":"sha256:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855","version":"0.1.0"}"#,
            r#","levers":[],"parents":[]"#,
            r#","policy":{"hash":"sha256:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855","source":"repo""#,
            r#","version":1},"signer":{"fingerprint":"SHA256:AAAA","principal":"dev@example.com"},"state":"covered""#,
            r#","tree":"sha1:4b825dc642cb6eb9a060e54bf8d69288fbee4904","ts":"2026-10-10T18:30:00Z""#,
            r#","unread":["test"],"v":3}"#
        );
        assert_eq!(json, want);
        assert_eq!(
            r.id().unwrap().to_string(),
            "sha256:992ac6676a2b1422dd7343434c5ab326f382e084249616c4e7ba1206fc74fad2"
        );
        assert_eq!(from_canonical::<Root>(json.as_bytes()).unwrap(), r);
        let merge = String::from_utf8(v3_merge().canonical_bytes().unwrap()).unwrap();
        assert!(
            merge.contains(r#""observed":[{"body":{"id":"test","observed":{"#),
            "{merge}"
        );
        assert_eq!(
            from_canonical::<Root>(merge.as_bytes()).unwrap(),
            v3_merge()
        );
    }
}
