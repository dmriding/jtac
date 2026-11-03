//! The release attestation (DESIGN.md section 16.3): one signed document per tag saying every
//! commit from the previous release to this one was covered or declared off by a trusted
//! root, naming those roots. Signed as a whole in `jtac-release-v1`; the roots it names are
//! verified on their own, so a reader re-derives the verdict rather than trusting the counts.

use serde::{Deserialize, Serialize};

use crate::canonical::{CanonicalError, canonical_bytes, canonical_digest};
use crate::digest::Sha256Digest;
use crate::oid::GitOid;
use crate::root::GateRef;
use crate::schema::V1;
use crate::signer::Signer;
use crate::time::Ts;

/// What a release attests.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Release {
    /// Schema version.
    pub v: V1,
    /// The tag's short name, `v0.2.8`.
    pub tag: String,
    /// The tag object itself when annotated, else the commit: a re-tag changes it.
    pub object: GitOid,
    /// The commit the tag points at.
    pub commit: GitOid,
    /// Its tree.
    pub tree: GitOid,
    /// Instant of the verdict.
    pub ts: Ts,
    /// Where the range starts: the previous release, or what `--from` named.
    pub from: From,
    /// Commits in `from..commit`.
    pub commits: u64,
    /// Of those, covered.
    pub covered: u64,
    /// Of those, declared off; listed in `waived`.
    pub declared_off: u64,
    /// Every commit declared off, newest first.
    pub waived: Vec<GitOid>,
    /// Every root that gave a commit its state, newest commit first, each once.
    pub roots: Vec<Sha256Digest>,
    /// Whether every first-parent commit was held to the CI namespace (`--require-ci`).
    pub ci: bool,
    /// The build outputs `--sums` named, in file order; empty when none were given.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub artifacts: Vec<Artifact>,
    /// The gate that produced this attestation.
    pub gate: GateRef,
    /// Who signed.
    pub signer: Signer,
}

/// The range's start.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct From {
    /// The commit excluded and everything behind it.
    pub commit: GitOid,
    /// Its tag, when the start was one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tag: Option<String>,
}

/// One line of `SHA256SUMS`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Artifact {
    /// The file name as the sums file spells it.
    pub name: String,
    /// Its digest.
    pub sha256: Sha256Digest,
}

impl Release {
    /// RFC 8785 bytes, the input to `id` and to the signature.
    pub fn canonical_bytes(&self) -> Result<Vec<u8>, CanonicalError> {
        canonical_bytes(self)
    }

    /// `sha256:` of the canonical bytes.
    pub fn id(&self) -> Result<Sha256Digest, CanonicalError> {
        canonical_digest(self)
    }

    /// Shape rules a reader checks before trusting any field: an attestation exists only for
    /// a range with nothing absent, so the counts must add up and the roots must be there.
    pub fn validate(&self) -> Result<(), ReleaseError> {
        let plain =
            |t: &str| !t.is_empty() && !t.chars().any(|c| c.is_control() || c.is_whitespace());
        if !plain(&self.tag) || !self.from.tag.as_deref().is_none_or(plain) {
            return Err(ReleaseError::Tag);
        }
        if self.from.commit == self.commit {
            return Err(ReleaseError::From);
        }
        if self.covered + self.declared_off != self.commits {
            return Err(ReleaseError::Counts);
        }
        if self.waived.len() as u64 != self.declared_off {
            return Err(ReleaseError::Waived);
        }
        if (self.commits > 0) != !self.roots.is_empty() {
            return Err(ReleaseError::Roots);
        }
        if self.roots.windows(2).any(|w| w[0] == w[1]) || !distinct(&self.roots) {
            return Err(ReleaseError::Roots);
        }
        if !distinct(&self.artifacts.iter().map(|a| &a.name).collect::<Vec<_>>()) {
            return Err(ReleaseError::Artifacts);
        }
        Ok(())
    }
}

fn distinct<T: Eq>(items: &[T]) -> bool {
    items
        .iter()
        .enumerate()
        .all(|(i, a)| items[..i].iter().all(|b| b != a))
}

/// Why a release attestation does not read.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ReleaseError {
    /// The tag, or the start's, is empty or holds whitespace or a control character.
    #[error("a tag name is empty or holds whitespace")]
    Tag,
    /// The start is the tag's own commit: nothing was judged.
    #[error("the range starts at the tag's own commit")]
    From,
    /// `covered + declared_off != commits`.
    #[error("covered plus declared off is not the commit count")]
    Counts,
    /// `waived` does not list exactly `declared_off` commits.
    #[error("`waived` does not list every declared-off commit")]
    Waived,
    /// No roots for a non-empty range, roots for an empty one, or a root listed twice.
    #[error("`roots` must name each root once and only for a range with commits")]
    Roots,
    /// An artifact name listed twice.
    #[error("an artifact is listed twice")]
    Artifacts,
}

#[cfg(test)]
mod tests {
    use super::*;

    const EMPTY: &str = "sha256:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";

    fn oid(n: u8) -> GitOid {
        format!("sha1:{n:040x}").parse().unwrap()
    }

    fn digest(n: u8) -> Sha256Digest {
        format!("sha256:{n:064x}").parse().unwrap()
    }

    fn release() -> Release {
        Release {
            v: V1,
            tag: "v0.2.8".into(),
            object: oid(9),
            commit: oid(2),
            tree: oid(3),
            ts: "2026-11-03T18:30:00Z".parse().unwrap(),
            from: From {
                commit: oid(1),
                tag: Some("v0.2.7".into()),
            },
            commits: 2,
            covered: 1,
            declared_off: 1,
            waived: vec![oid(4)],
            roots: vec![digest(1), digest(2)],
            ci: true,
            artifacts: Vec::new(),
            gate: GateRef {
                version: "0.2.8".into(),
                binary: EMPTY.parse().unwrap(),
            },
            signer: Signer {
                principal: "ci@example.com".into(),
                fingerprint: "SHA256:AAAA".into(),
            },
        }
    }

    /// Canonical bytes fixed for the schema: a reader on any machine computes this id.
    #[test]
    fn release_v1_id_known_answer() {
        let r = release();
        let bytes = String::from_utf8(r.canonical_bytes().unwrap()).unwrap();
        assert_eq!(
            bytes,
            "{\"ci\":true,\"commit\":\"sha1:0000000000000000000000000000000000000002\",\
             \"commits\":2,\"covered\":1,\"declared_off\":1,\"from\":{\"commit\":\
             \"sha1:0000000000000000000000000000000000000001\",\"tag\":\"v0.2.7\"},\
             \"gate\":{\"binary\":\"sha256:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855\",\
             \"version\":\"0.2.8\"},\"object\":\"sha1:0000000000000000000000000000000000000009\",\
             \"roots\":[\"sha256:0000000000000000000000000000000000000000000000000000000000000001\",\
             \"sha256:0000000000000000000000000000000000000000000000000000000000000002\"],\
             \"signer\":{\"fingerprint\":\"SHA256:AAAA\",\"principal\":\"ci@example.com\"},\
             \"tag\":\"v0.2.8\",\"tree\":\"sha1:0000000000000000000000000000000000000003\",\
             \"ts\":\"2026-11-03T18:30:00Z\",\"v\":1,\"waived\":\
             [\"sha1:0000000000000000000000000000000000000004\"]}"
        );
        assert_eq!(
            r.id().unwrap().to_string(),
            "sha256:1749eb37d6fd50d36386929675a3241ed32da93eea6aec2a5d0ce2ef8246d4f9"
        );
        let back: Release = crate::from_canonical(r.canonical_bytes().unwrap().as_slice()).unwrap();
        assert_eq!(back, r);
    }

    #[test]
    fn artifacts_and_from_tag_are_absent_from_the_bytes_when_unset() {
        let mut r = release();
        r.from.tag = None;
        let bytes = String::from_utf8(r.canonical_bytes().unwrap()).unwrap();
        assert!(!bytes.contains("artifacts"), "{bytes}");
        assert!(!bytes.contains("\"tag\":\"v0.2.7\""), "{bytes}");
        r.artifacts.push(Artifact {
            name: "jtac-v0.2.8-x86_64-unknown-linux-musl.tar.gz".into(),
            sha256: digest(7),
        });
        let bytes = String::from_utf8(r.canonical_bytes().unwrap()).unwrap();
        assert!(bytes.contains("\"artifacts\":[{\"name\":"), "{bytes}");
    }

    #[test]
    fn a_release_with_an_unknown_field_or_another_version_does_not_read() {
        let mut v: serde_json::Value =
            serde_json::from_slice(&release().canonical_bytes().unwrap()).unwrap();
        v["extra"] = serde_json::json!(1);
        assert!(serde_json::from_value::<Release>(v.clone()).is_err());
        v.as_object_mut().unwrap().remove("extra");
        v["v"] = serde_json::json!(2);
        assert!(serde_json::from_value::<Release>(v).is_err());
    }

    #[test]
    fn validate_refuses_each_shape_a_writer_never_produces() {
        assert_eq!(release().validate(), Ok(()));
        type Edit = fn(&mut Release);
        let rows: [(Edit, ReleaseError); 9] = [
            (|r| r.tag = String::new(), ReleaseError::Tag),
            (|r| r.from.commit = r.commit, ReleaseError::From),
            (|r| r.tag = "v0 .2".into(), ReleaseError::Tag),
            (|r| r.from.tag = Some("v1\n".into()), ReleaseError::Tag),
            (|r| r.covered = 2, ReleaseError::Counts),
            (|r| r.waived.clear(), ReleaseError::Waived),
            (|r| r.roots.clear(), ReleaseError::Roots),
            (|r| r.roots.push(r.roots[0]), ReleaseError::Roots),
            (
                |r| {
                    let a = Artifact {
                        name: "x".into(),
                        sha256: r.roots[0],
                    };
                    r.artifacts = vec![a.clone(), a];
                },
                ReleaseError::Artifacts,
            ),
        ];
        for (edit, want) in rows {
            let mut r = release();
            edit(&mut r);
            assert_eq!(r.validate(), Err(want));
        }
        let mut empty = release();
        empty.commits = 0;
        empty.covered = 0;
        empty.declared_off = 0;
        empty.waived.clear();
        assert_eq!(empty.validate(), Err(ReleaseError::Roots));
        empty.roots.clear();
        assert_eq!(empty.validate(), Ok(()));
    }
}
