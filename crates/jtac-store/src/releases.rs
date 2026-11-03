//! The release ref: one parentless commit per attested tag at `refs/jtac/releases/<tag>`,
//! its tree `release.json` and `release.sig`, written as a root's is (DESIGN.md section
//! 16.3). Under `refs/jtac/` so the fetch refspec `init` adds carries it to every clone;
//! outside `roots/` so the root listing never reads it. Created, never replaced: a tag is
//! attested once, as it is tagged once.

use jtac_checks::git::{GitError, git};
use jtac_core::{Release, Sha256Digest, from_canonical};

use crate::refs::{commit_files, document, kind_of};
use crate::{Pushed, Store, StoreError};

const NAMESPACE: &str = "refs/jtac/releases/";

/// The ref that holds the attestation for `tag`.
pub fn release_ref(tag: &str) -> String {
    format!("{NAMESPACE}{tag}")
}

/// An attestation as a ref holds it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StoredRelease {
    /// The document.
    pub release: Release,
    /// SSHSIG over its canonical bytes, as stored.
    pub sig: Vec<u8>,
}

impl Store {
    /// Write the attestation commit and create its ref; the attestation's id. Refuses one that
    /// would not read back: fails `validate`, names a tag object or commit this repository does
    /// not have, or a tag whose ref name git refuses. A ref that already exists is `Exists`.
    pub fn put_release(&self, release: &Release, sig: &[u8]) -> Result<Sha256Digest, StoreError> {
        let invalid = |what: String| StoreError::Invalid { what };
        release.validate().map_err(|e| invalid(e.to_string()))?;
        let name = release_ref(&release.tag);
        if git(self.repo(), &["check-ref-format", &name]).is_err() {
            return Err(invalid(format!(
                "`{}` is not a tag name git accepts",
                release.tag
            )));
        }
        for (what, oid) in [("tag object", &release.object), ("commit", &release.commit)] {
            if kind_of(self, &oid.hex())?.is_none() {
                return Err(invalid(format!(
                    "the {what} {oid} is not in this repository"
                )));
            }
        }
        let id = release.id()?;
        let files = [
            ("release.json", release.canonical_bytes()?),
            ("release.sig", sig.to_vec()),
        ];
        let commit = commit_files(
            self,
            &files,
            &release.signer.principal,
            release.ts,
            &id.to_string(),
        )?;
        match git(self.repo(), &["update-ref", &name, &commit, ""]) {
            Ok(_) => Ok(id),
            Err(GitError::Failed { .. }) if self.release(&release.tag)?.is_some() => {
                Err(StoreError::Exists { name })
            }
            Err(e) => Err(e.into()),
        }
    }

    /// The attestation at `tag`'s ref, or `None` when there is no such ref. A ref that holds
    /// anything else is refused, never a miss.
    pub fn release(&self, tag: &str) -> Result<Option<StoredRelease>, StoreError> {
        let name = release_ref(tag);
        let Some(doc) = document(self, &name, "release")? else {
            return Ok(None);
        };
        let refuse = |what: String| StoreError::Ref {
            name: name.clone(),
            what,
        };
        let release: Release =
            from_canonical(&doc.blob("release.json")?).map_err(|e| refuse(e.to_string()))?;
        release.validate().map_err(|e| refuse(e.to_string()))?;
        if release.tag != tag {
            return Err(refuse(format!("the attestation is for {}", release.tag)));
        }
        let sig = doc.blob("release.sig")?;
        Ok(Some(StoredRelease { release, sig }))
    }

    /// The tag of every ref under the namespace, in ref order, read by `release` one at a
    /// time so a ref that holds no attestation is the caller's to skip or refuse.
    pub fn release_tags(&self) -> Result<Vec<String>, StoreError> {
        let out = git(
            self.repo(),
            &["for-each-ref", "--format=%(refname)", NAMESPACE],
        )?;
        let text = String::from_utf8(out).map_err(|_| StoreError::Output {
            what: "a release ref name is not UTF-8".into(),
        })?;
        Ok(text
            .lines()
            .filter_map(|name| name.strip_prefix(NAMESPACE))
            .map(str::to_owned)
            .collect())
    }

    /// Push `tag`'s attestation ref to `remote`, never forced.
    pub fn push_release(&self, remote: &str, tag: &str) -> Result<Pushed, StoreError> {
        let mut pushed = self.push_refs(remote, &[release_ref(tag)], &[])?;
        pushed.pop().ok_or_else(|| StoreError::Output {
            what: "push --porcelain reported no ref".into(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_release_ref_sits_beside_the_roots_namespace_not_in_it() {
        assert_eq!(release_ref("v0.2.8"), "refs/jtac/releases/v0.2.8");
        assert!(!release_ref("v1").starts_with("refs/jtac/roots/"));
    }
}
