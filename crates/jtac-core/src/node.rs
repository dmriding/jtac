//! Evidence nodes: one check run, one finding, or one lever pulled (DESIGN.md section 9.2).

use jiff::Timestamp;
use serde::de::Error as _;
use serde::ser::Error as _;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::canonical::{CanonicalError, canonical_bytes, canonical_digest};
use crate::digest::Sha256Digest;
use crate::oid::GitOid;
use crate::schema::V1;
use crate::signer::Signer;

/// One piece of evidence about one tree under one policy.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Node {
    /// The git tree the evidence is about.
    pub tree: GitOid,
    /// Hash of the effective policy (DESIGN.md section 10.1).
    pub policy: Sha256Digest,
    /// Informational. The verdict reads the root's `ts`, never a node's.
    pub ts: Timestamp,
    /// The kind-specific body.
    pub body: Body,
}

impl Node {
    /// The kind the body carries.
    pub fn kind(&self) -> Kind {
        match self.body {
            Body::Check(_) => Kind::Check,
            Body::Finding(_) => Kind::Finding,
            Body::Lever(_) => Kind::Lever,
        }
    }

    /// RFC 8785 bytes, the input to `id`.
    pub fn canonical_bytes(&self) -> Result<Vec<u8>, CanonicalError> {
        canonical_bytes(self)
    }

    /// `sha256:` of the canonical bytes.
    pub fn id(&self) -> Result<Sha256Digest, CanonicalError> {
        canonical_digest(self)
    }
}

/// The node kinds v1 writes. Later kinds are reserved in DESIGN.md and refused here.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    /// A configured check ran.
    Check,
    /// A check reported something.
    Finding,
    /// A person changed the gate: init, waive, policy or trust change.
    Lever,
}

/// Kind-specific fields.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Body {
    /// See [`CheckBody`].
    Check(CheckBody),
    /// See [`FindingBody`].
    Finding(FindingBody),
    /// See [`LeverBody`].
    Lever(LeverBody),
}

/// One run of one configured check.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CheckBody {
    /// The check id from the policy.
    pub id: String,
    /// The argv that ran, never a shell string.
    pub argv: Vec<String>,
    /// Version the tool reported.
    pub tool_version: String,
    /// Process exit code.
    pub exit: i32,
    /// What the gate concluded from the run.
    pub outcome: Outcome,
    /// Digest of the captured stdout blob.
    pub stdout_sha: Sha256Digest,
    /// Digest of the captured stderr blob.
    pub stderr_sha: Sha256Digest,
    /// Wall time of the run.
    pub duration_ms: u64,
    /// True when served from the check cache.
    pub cached: bool,
}

/// How a check run ended. `skipped` and `error` are not `pass`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    /// The check ran and passed.
    Pass,
    /// The check ran and failed.
    Fail,
    /// The check did not run on purpose.
    Skipped,
    /// The check could not run or could not be read.
    Error,
}

/// One thing a check reported.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FindingBody {
    /// Id of the check node that produced it.
    pub check: Sha256Digest,
    /// Rule id, for example `JT-TEST-003`.
    pub rule: String,
    /// How bad.
    pub severity: Severity,
    /// Repository-relative path.
    pub path: String,
    /// One-based line, zero when the finding has no line.
    pub line: u64,
    /// Human-readable detail.
    pub message: String,
}

/// Finding severity. The policy names the lowest one that blocks.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    /// Never ships.
    Blocker,
    /// Blocks by default.
    Major,
    /// Recorded, does not block.
    Minor,
}

/// A deliberate, signed change to the gate.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LeverBody {
    /// What was done.
    pub action: Action,
    /// Who did it.
    pub signer: Signer,
    /// The reason given, verbatim.
    pub reason: String,
    /// What a waiver covers. Absent for actions that have no scope.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope: Option<Scope>,
    /// When a waiver stops applying, compared against the root's `ts`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires: Option<Timestamp>,
}

/// Lever actions.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Action {
    /// `jtac init` ran.
    Init,
    /// `jtac waive` ran.
    Waive,
    /// The policy changed.
    PolicyChange,
    /// The trust file changed.
    TrustChange,
}

/// What a waiver covers: one commit, or one rule on one path (DESIGN.md section 11.6).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Scope {
    /// The whole commit.
    Commit(GitOid),
    /// One rule under one path.
    Rule {
        /// Rule id.
        rule: String,
        /// Repository-relative path prefix.
        path: String,
    },
}

// The wire form: `kind` and `body` side by side at the top level, `v` checked, no extra keys.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Wire {
    v: V1,
    kind: Kind,
    tree: GitOid,
    policy: Sha256Digest,
    ts: Timestamp,
    body: serde_json::Value,
}

impl Serialize for Node {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let body = match &self.body {
            Body::Check(b) => serde_json::to_value(b),
            Body::Finding(b) => serde_json::to_value(b),
            Body::Lever(b) => serde_json::to_value(b),
        }
        .map_err(S::Error::custom)?;
        Wire {
            v: V1,
            kind: self.kind(),
            tree: self.tree,
            policy: self.policy,
            ts: self.ts,
            body,
        }
        .serialize(s)
    }
}

impl<'de> Deserialize<'de> for Node {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let w = Wire::deserialize(d)?;
        let body = match w.kind {
            Kind::Check => serde_json::from_value(w.body).map(Body::Check),
            Kind::Finding => serde_json::from_value(w.body).map(Body::Finding),
            Kind::Lever => serde_json::from_value(w.body).map(Body::Lever),
        }
        .map_err(D::Error::custom)?;
        Ok(Node {
            tree: w.tree,
            policy: w.policy,
            ts: w.ts,
            body,
        })
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub(crate) const EMPTY: &str =
        "sha256:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";
    pub(crate) const TS: &str = "2026-10-10T18:30:00Z";

    pub(crate) fn check_node() -> Node {
        Node {
            tree: GitOid::EMPTY_TREE_SHA1.parse().unwrap(),
            policy: EMPTY.parse().unwrap(),
            ts: TS.parse().unwrap(),
            body: Body::Check(CheckBody {
                id: "cargo-test".into(),
                argv: vec!["cargo".into(), "test".into(), "--workspace".into()],
                tool_version: "1.98.0".into(),
                exit: 0,
                outcome: Outcome::Pass,
                stdout_sha: EMPTY.parse().unwrap(),
                stderr_sha: EMPTY.parse().unwrap(),
                duration_ms: 1234,
                cached: false,
            }),
        }
    }

    pub(crate) fn lever_node(action: Action) -> Node {
        Node {
            body: Body::Lever(LeverBody {
                action,
                signer: Signer {
                    principal: "dev@example.com".into(),
                    fingerprint: "SHA256:AAAA".into(),
                },
                reason: "example reason".into(),
                scope: Some(Scope::Rule {
                    rule: "JT-SUP-001".into(),
                    path: "src/ffi/".into(),
                }),
                expires: Some("2026-10-24T18:30:00Z".parse().unwrap()),
            }),
            ..check_node()
        }
    }

    fn finding_node() -> Node {
        Node {
            body: Body::Finding(FindingBody {
                check: EMPTY.parse().unwrap(),
                rule: "JT-TEST-003".into(),
                severity: Severity::Major,
                path: "tests/example.rs".into(),
                line: 12,
                message: "assertion removed".into(),
            }),
            ..check_node()
        }
    }

    #[test]
    fn node_id_known_answer() {
        // DESIGN.md section 9.2. Changing these bytes changes every id ever written.
        let want = concat!(
            r#"{"body":{"argv":["cargo","test","--workspace"],"cached":false,"duration_ms":1234,"exit":0,"id":"cargo-test","outcome":"pass","#,
            r#""stderr_sha":"sha256:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855","#,
            r#""stdout_sha":"sha256:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855","tool_version":"1.98.0"},"#,
            r#""kind":"check","policy":"sha256:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855","#,
            r#""tree":"sha1:4b825dc642cb6eb9a060e54bf8d69288fbee4904","ts":"2026-10-10T18:30:00Z","v":1}"#
        );
        let node = check_node();
        assert_eq!(
            String::from_utf8(node.canonical_bytes().unwrap()).unwrap(),
            want
        );
        assert_eq!(
            node.id().unwrap().to_string(),
            "sha256:34bb1ab4a26a881ab4c03234fe6ac2a1a618f2d501bfeac4a8c874d69fae3209"
        );
    }

    #[test]
    fn every_kind_round_trips_through_json() {
        for node in [check_node(), finding_node(), lever_node(Action::Waive)] {
            let json = serde_json::to_string(&node).unwrap();
            let back: Node = serde_json::from_str(&json).unwrap();
            assert_eq!(back, node);
            assert_eq!(back.kind(), node.kind());
        }
    }

    #[test]
    fn absent_scope_and_expiry_are_omitted_not_null() {
        let mut node = lever_node(Action::Init);
        if let Body::Lever(b) = &mut node.body {
            b.scope = None;
            b.expires = None;
        }
        let json = serde_json::to_string(&node).unwrap();
        assert!(
            !json.contains("scope") && !json.contains("expires"),
            "{json}"
        );
        assert_eq!(serde_json::from_str::<Node>(&json).unwrap(), node);
    }

    #[test]
    fn scope_spellings() {
        let commit = Scope::Commit(GitOid::EMPTY_TREE_SHA1.parse().unwrap());
        assert_eq!(
            serde_json::to_string(&commit).unwrap(),
            r#"{"commit":"sha1:4b825dc642cb6eb9a060e54bf8d69288fbee4904"}"#
        );
        let rule = Scope::Rule {
            rule: "JT-SUP-001".into(),
            path: "src/".into(),
        };
        assert_eq!(
            serde_json::to_string(&rule).unwrap(),
            r#"{"rule":{"rule":"JT-SUP-001","path":"src/"}}"#
        );
    }

    #[test]
    fn wire_refuses_what_v1_did_not_write() {
        let good = serde_json::to_value(check_node()).unwrap();
        let mut extra = good.clone();
        extra["extra"] = 1.into();
        let mut extra_in_body = good.clone();
        extra_in_body["body"]["extra"] = 1.into();
        let mut v2 = good.clone();
        v2["v"] = 2.into();
        let mut wrong_kind = good.clone();
        wrong_kind["kind"] = "finding".into();
        let mut unknown_kind = good.clone();
        unknown_kind["kind"] = "claim".into();
        let mut bad_outcome = good.clone();
        bad_outcome["body"]["outcome"] = "ok".into();
        let mut missing = good.clone();
        missing.as_object_mut().unwrap().remove("ts");
        for (name, doc) in [
            ("extra top-level key", extra),
            ("extra body key", extra_in_body),
            ("version 2", v2),
            ("kind does not match body", wrong_kind),
            ("reserved kind", unknown_kind),
            ("unknown outcome", bad_outcome),
            ("missing ts", missing),
        ] {
            assert!(
                serde_json::from_value::<Node>(doc).is_err(),
                "{name} was accepted"
            );
        }
        assert!(serde_json::from_value::<Node>(good).is_ok());
    }

    #[test]
    fn severity_orders_blocker_first() {
        assert!(Severity::Blocker < Severity::Major && Severity::Major < Severity::Minor);
    }
}
