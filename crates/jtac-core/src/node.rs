//! Evidence nodes: one check run, one finding, or one lever pulled (DESIGN.md section 9.2).

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::canonical::{CanonicalError, canonical_bytes, canonical_digest};
use crate::digest::Sha256Digest;
use crate::oid::GitOid;
use crate::schema::V1;
use crate::signer::Signer;
use crate::time::Ts;

/// One piece of evidence about one tree under one policy.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Node {
    /// The git tree the evidence is about.
    pub tree: GitOid,
    /// Hash of the effective policy (DESIGN.md section 10.1).
    pub policy: Sha256Digest,
    /// Informational, except a waiver's: its 30-day cap runs from here (DESIGN.md 11.6).
    pub ts: Ts,
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
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(untagged)]
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
    /// Captured stdout.
    pub stdout: Capture,
    /// Captured stderr.
    pub stderr: Capture,
    /// Wall time of the run.
    pub duration_ms: u64,
    /// Id of the node this result was cached from. Absent on a fresh run.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cached_from: Option<Sha256Digest>,
}

/// One captured output stream. `truncated` is what tells a cut capture from a short one.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Capture {
    /// Digest of the stored blob.
    pub sha: Sha256Digest,
    /// Bytes stored.
    pub bytes: u64,
    /// True when the capture cap cut the stream.
    pub truncated: bool,
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
    pub expires: Option<Ts>,
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
#[serde(untagged, deny_unknown_fields)]
pub enum Scope {
    /// The whole commit.
    Commit {
        /// The commit.
        commit: GitOid,
    },
    /// One rule under one path.
    Rule {
        /// Rule id.
        rule: String,
        /// Repository-relative path prefix.
        path: String,
    },
}

// `kind` and `body` side by side at the top level. Not `flatten`: that silently disables
// `deny_unknown_fields`, which is the whole point of the reader.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Wire<B> {
    v: V1,
    kind: Kind,
    tree: GitOid,
    policy: Sha256Digest,
    ts: Ts,
    body: B,
}

impl Serialize for Node {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        Wire {
            v: V1,
            kind: self.kind(),
            tree: self.tree,
            policy: self.policy,
            ts: self.ts,
            body: &self.body,
        }
        .serialize(s)
    }
}

impl<'de> Deserialize<'de> for Node {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let w = Wire::<serde_json::Value>::deserialize(d)?;
        let body = match w.kind {
            Kind::Check => serde_json::from_value(w.body).map(Body::Check),
            Kind::Finding => serde_json::from_value(w.body).map(Body::Finding),
            Kind::Lever => serde_json::from_value(w.body).map(Body::Lever),
        }
        .map_err(serde::de::Error::custom)?;
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
    use crate::canonical::{ReadError, from_canonical};

    pub(crate) const EMPTY: &str =
        "sha256:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";
    pub(crate) const TS: &str = "2026-10-10T18:30:00Z";
    pub(crate) const COMMIT_ONE: &str = "sha1:0000000000000000000000000000000000000001";

    pub(crate) fn signer() -> Signer {
        Signer {
            principal: "dev@example.com".into(),
            fingerprint: "SHA256:AAAA".into(),
        }
    }

    fn empty_capture() -> Capture {
        Capture {
            sha: EMPTY.parse().unwrap(),
            bytes: 0,
            truncated: false,
        }
    }

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
                stdout: empty_capture(),
                stderr: empty_capture(),
                duration_ms: 1234,
                cached_from: None,
            }),
        }
    }

    pub(crate) fn waiver(scope: Scope, expires: Option<&str>) -> Node {
        Node {
            body: Body::Lever(LeverBody {
                action: Action::Waive,
                signer: signer(),
                reason: "example reason".into(),
                scope: Some(scope),
                expires: expires.map(|e| e.parse().unwrap()),
            }),
            ..check_node()
        }
    }

    pub(crate) fn commit_waiver() -> Node {
        waiver(
            Scope::Commit {
                commit: COMMIT_ONE.parse().unwrap(),
            },
            None,
        )
    }

    pub(crate) fn rule_waiver() -> Node {
        waiver(
            Scope::Rule {
                rule: "JT-SUP-001".into(),
                path: "src/ffi/".into(),
            },
            Some("2026-10-24T18:30:00Z"),
        )
    }

    pub(crate) fn lever(action: Action) -> Node {
        let mut node = commit_waiver();
        if let Body::Lever(b) = &mut node.body {
            b.action = action;
            b.scope = None;
        }
        node
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
            r#"{"body":{"argv":["cargo","test","--workspace"],"duration_ms":1234,"exit":0,"id":"cargo-test","outcome":"pass","#,
            r#""stderr":{"bytes":0,"sha":"sha256:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855","truncated":false},"#,
            r#""stdout":{"bytes":0,"sha":"sha256:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855","truncated":false},"#,
            r#""tool_version":"1.98.0"},"kind":"check","policy":"sha256:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855","#,
            r#""tree":"sha1:4b825dc642cb6eb9a060e54bf8d69288fbee4904","ts":"2026-10-10T18:30:00Z","v":1}"#
        );
        let node = check_node();
        assert_eq!(
            String::from_utf8(node.canonical_bytes().unwrap()).unwrap(),
            want
        );
        assert_eq!(
            node.id().unwrap().to_string(),
            "sha256:7dbd2c1d6519975840d8a0b6df2df44e9c32b9aff38102eabc5554058cb5696e"
        );
        assert_eq!(from_canonical::<Node>(want.as_bytes()).unwrap(), node);
    }

    #[test]
    fn every_kind_round_trips_through_canonical_bytes() {
        for node in [
            check_node(),
            finding_node(),
            commit_waiver(),
            rule_waiver(),
            lever(Action::Init),
        ] {
            let bytes = node.canonical_bytes().unwrap();
            let back: Node = from_canonical(&bytes).unwrap();
            assert_eq!(back, node);
            assert_eq!(back.kind(), node.kind());
        }
    }

    #[test]
    fn wire_spellings_table() {
        let cases: [(&str, serde_json::Value); 14] = [
            ("pass", serde_json::to_value(Outcome::Pass).unwrap()),
            ("fail", serde_json::to_value(Outcome::Fail).unwrap()),
            ("skipped", serde_json::to_value(Outcome::Skipped).unwrap()),
            ("error", serde_json::to_value(Outcome::Error).unwrap()),
            ("blocker", serde_json::to_value(Severity::Blocker).unwrap()),
            ("major", serde_json::to_value(Severity::Major).unwrap()),
            ("minor", serde_json::to_value(Severity::Minor).unwrap()),
            ("init", serde_json::to_value(Action::Init).unwrap()),
            ("waive", serde_json::to_value(Action::Waive).unwrap()),
            (
                "policy_change",
                serde_json::to_value(Action::PolicyChange).unwrap(),
            ),
            (
                "trust_change",
                serde_json::to_value(Action::TrustChange).unwrap(),
            ),
            ("check", serde_json::to_value(Kind::Check).unwrap()),
            ("finding", serde_json::to_value(Kind::Finding).unwrap()),
            ("lever", serde_json::to_value(Kind::Lever).unwrap()),
        ];
        for (want, got) in cases {
            assert_eq!(got, serde_json::Value::String(want.into()));
        }
        let commit = Scope::Commit {
            commit: COMMIT_ONE.parse().unwrap(),
        };
        assert_eq!(
            serde_json::to_string(&commit).unwrap(),
            r#"{"commit":"sha1:0000000000000000000000000000000000000001"}"#
        );
        let rule = Scope::Rule {
            rule: "JT-SUP-001".into(),
            path: "src/".into(),
        };
        assert_eq!(
            serde_json::to_string(&rule).unwrap(),
            r#"{"rule":"JT-SUP-001","path":"src/"}"#
        );
    }

    #[test]
    fn absent_optional_fields_are_omitted_never_null() {
        let node = lever(Action::Init);
        let json = String::from_utf8(node.canonical_bytes().unwrap()).unwrap();
        assert!(
            !json.contains("scope") && !json.contains("expires") && !json.contains("null"),
            "{json}"
        );
        let check = String::from_utf8(check_node().canonical_bytes().unwrap()).unwrap();
        assert!(!check.contains("cached_from"), "{check}");
        let null_scope = json.replace(r#""reason""#, r#""expires":null,"reason""#);
        assert!(matches!(
            from_canonical::<Node>(null_scope.as_bytes()),
            Err(ReadError::NotCanonical)
        ));
    }

    #[test]
    fn wire_refuses_what_v1_did_not_write() {
        let good = serde_json::to_value(check_node()).unwrap();
        let mut extra = good.clone();
        extra["extra"] = 1.into();
        let mut extra_in_body = good.clone();
        extra_in_body["body"]["extra"] = 1.into();
        let mut extra_in_capture = good.clone();
        extra_in_capture["body"]["stdout"]["cap"] = 1.into();
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
        let mut offset_ts = good.clone();
        offset_ts["ts"] = "2026-10-10T20:30:00+02:00".into();
        let mut scope_extra = serde_json::to_value(rule_waiver()).unwrap();
        scope_extra["body"]["scope"]["approved_by"] = "x".into();
        let mut scope_both = serde_json::to_value(rule_waiver()).unwrap();
        scope_both["body"]["scope"]["commit"] = COMMIT_ONE.into();
        for (name, doc) in [
            ("extra top-level key", extra),
            ("extra body key", extra_in_body),
            ("extra capture key", extra_in_capture),
            ("version 2", v2),
            ("kind does not match body", wrong_kind),
            ("reserved kind", unknown_kind),
            ("unknown outcome", bad_outcome),
            ("missing ts", missing),
            ("ts with an offset", offset_ts),
            ("extra scope key", scope_extra),
            ("scope with both variants", scope_both),
        ] {
            assert!(
                serde_json::from_value::<Node>(doc).is_err(),
                "{name} was accepted"
            );
        }
        assert!(serde_json::from_value::<Node>(good).is_ok());
    }

    #[test]
    fn duplicate_body_keys_are_not_canonical() {
        let json = String::from_utf8(commit_waiver().canonical_bytes().unwrap()).unwrap();
        let dup = json.replace(r#""action":"waive""#, r#""action":"init","action":"waive""#);
        assert!(
            serde_json::from_str::<Node>(&dup).is_ok(),
            "serde_json alone keeps the last key"
        );
        assert!(matches!(
            from_canonical::<Node>(dup.as_bytes()),
            Err(ReadError::NotCanonical)
        ));
    }

    #[test]
    fn an_integer_past_2_pow_53_parses_but_has_no_id() {
        let mut doc = serde_json::to_value(check_node()).unwrap();
        doc["body"]["duration_ms"] = u64::MAX.into();
        let node: Node = serde_json::from_value(doc).unwrap();
        let err = node.id().unwrap_err();
        assert!(
            matches!(err, CanonicalError::UnsafeInteger { ref path } if path == ".body.duration_ms"),
            "{err}"
        );
    }

    #[test]
    fn severity_orders_blocker_first() {
        assert!(Severity::Blocker < Severity::Major && Severity::Major < Severity::Minor);
    }
}
