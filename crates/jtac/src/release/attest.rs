//! The attestation written and judged (DESIGN.md section 16.3): built from the verdict,
//! signed in `jtac-release-v1`, trusted only under a signers line that names the namespace,
//! stored at the tag's ref once. An attestation the tag already has stands when it is
//! trusted and this run re-derives it: same tag object, same roots, same start, same
//! artifacts where both name them. A rerun of the release job then changes nothing.

use std::path::Path;

use jtac_core::{Artifact, From, GateRef, Release, Sha256Digest, Signer, Ts, V1};
use jtac_gate::ReleaseVerdict;
use jtac_sign::{Identity, Namespace, Verified, sign, verify};
use jtac_store::{PushOutcome, Store, StoredRelease, release_ref};
use serde::Serialize;

use super::facts::Tag;
use crate::coverage;
use crate::refuse::Refusal;
use crate::trust::Signers;

/// The attestation as shown, whichever way it was reached.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Attested {
    /// The document's id.
    pub id: Sha256Digest,
    /// Who signed.
    pub signer: Signer,
    /// `trusted`, `unknown` or `invalid`.
    pub trust: &'static str,
    /// Why this run's verdict is not what the document says, when it is not.
    pub disagrees: Option<String>,
    /// The ref.
    pub reference: String,
    /// `written` by this run, or `stands` from before.
    pub origin: &'static str,
    /// What the push did: `sent`, `up to date`, or `rejected: <summary>`.
    pub pushed: Option<String>,
}

impl Attested {
    /// Trusted, agreeing, and on the remote when a push was asked for.
    pub fn holds(&self) -> bool {
        self.trust == "trusted"
            && self.disagrees.is_none()
            && !self
                .pushed
                .as_deref()
                .is_some_and(|p| p.starts_with("rejected"))
    }
}

/// What this run established, to build a document from or hold one against.
pub struct Derived<'a> {
    pub tag: &'a Tag,
    pub from: &'a From,
    pub commits: u64,
    pub verdict: &'a ReleaseVerdict,
    pub require_ci: bool,
    pub artifacts: &'a [Artifact],
}

/// The document this run would sign.
pub fn build(d: &Derived<'_>, now: Ts, gate: GateRef, signer: Signer) -> Release {
    Release {
        v: V1,
        tag: d.tag.name.clone(),
        object: d.tag.object,
        commit: d.tag.head.commit,
        tree: d.tag.head.tree,
        ts: now,
        from: d.from.clone(),
        commits: d.commits,
        covered: d.commits - d.verdict.waived.len() as u64,
        declared_off: d.verdict.waived.len() as u64,
        waived: d.verdict.waived.clone(),
        roots: d.verdict.roots.clone(),
        ci: d.require_ci,
        artifacts: d.artifacts.to_vec(),
        gate,
        signer,
    }
}

/// Sign, refuse a key the tag's trust does not name for the namespace, store, push.
pub fn attest(
    repo: &Path,
    store: &Store,
    id: &Identity,
    release: &Release,
    signers: &Signers,
    remote: &str,
) -> Result<Attested, Refusal> {
    let bytes = release.canonical_bytes().map_err(Refusal::internal)?;
    let sig = sign(id, Namespace::Release, &bytes).map_err(|e| Refusal::sign(&e))?;
    let tool = coverage::verifier(repo)?;
    let verified = verify(
        &tool,
        signers.as_sign(),
        &release.signer,
        Namespace::Release,
        release.ts,
        &bytes,
        &sig,
    )
    .map_err(|e| signers.refusal("release", &e))?;
    if verified != Verified::Trusted {
        return Err(Refusal::release_untrusted(
            &release.tag,
            &release.signer,
            &signers.describe(),
        ));
    }
    let id = store
        .put_release(release, sig.as_bytes())
        .map_err(Refusal::store)?;
    let mut out = Attested {
        id,
        signer: release.signer.clone(),
        trust: "trusted",
        disagrees: None,
        reference: release_ref(&release.tag),
        origin: "written",
        pushed: None,
    };
    out.pushed = Some(push(store, remote, &release.tag)?);
    Ok(out)
}

/// What the trust in force says of a stored attestation: `trusted`, `unknown` or
/// `invalid`. The one judgement a start, a standing attestation and `--verify` share.
pub fn trust_of(
    repo: &Path,
    signers: &Signers,
    stored: &StoredRelease,
) -> Result<&'static str, Refusal> {
    let r = &stored.release;
    let bytes = r.canonical_bytes().map_err(Refusal::internal)?;
    let tool = coverage::verifier(repo)?;
    let sig = String::from_utf8_lossy(&stored.sig);
    let verified = verify(
        &tool,
        signers.as_sign(),
        &r.signer,
        Namespace::Release,
        r.ts,
        &bytes,
        &sig,
    )
    .map_err(|e| signers.refusal("release", &e))?;
    Ok(match verified {
        Verified::Trusted => "trusted",
        Verified::Unknown { .. } => "unknown",
        Verified::Invalid { .. } => "invalid",
    })
}

/// Why a document's named start is not a trusted attestation in this clone, if it is not.
/// A `from.tag` is written only for a start that was one, so a reader holds it to one.
fn start_problem(
    repo: &Path,
    store: &Store,
    signers: &Signers,
    from: &From,
) -> Result<Option<String>, Refusal> {
    let Some(tag) = &from.tag else {
        return Ok(None);
    };
    let stored = match store.release(tag) {
        Ok(Some(stored)) => stored,
        Ok(None) => {
            return Ok(Some(format!(
                "its start {tag} is not an attested release in this clone"
            )));
        }
        Err(e) => return Ok(Some(format!("its start {tag} does not read: {e}"))),
    };
    if trust_of(repo, signers, &stored)? != "trusted" {
        return Ok(Some(format!(
            "its start {tag} is not a trusted attestation"
        )));
    }
    if stored.release.commit != from.commit {
        return Ok(Some(format!(
            "its start {tag} is attested at {}, not {}",
            stored.release.commit, from.commit
        )));
    }
    Ok(None)
}

/// Judge the attestation the tag already has against this run, and push it when asked.
pub fn judge(
    repo: &Path,
    store: &Store,
    stored: &StoredRelease,
    signers: &Signers,
    d: &Derived<'_>,
    remote: Option<&str>,
) -> Result<Attested, Refusal> {
    let r = &stored.release;
    let trust = trust_of(repo, signers, stored)?;
    let start = match trust {
        "trusted" => start_problem(repo, store, signers, &r.from)?,
        _ => None,
    };
    let disagrees = start.or_else(|| disagreement(r, d));
    let mut out = Attested {
        id: r.id().map_err(Refusal::internal)?,
        signer: r.signer.clone(),
        trust,
        disagrees,
        reference: release_ref(&r.tag),
        origin: "stands",
        pushed: None,
    };
    if let (Some(remote), true) = (remote, out.holds()) {
        out.pushed = Some(push(store, remote, &r.tag)?);
    }
    Ok(out)
}

/// The first way the document and this run part, if any: every field a reader could take
/// on faith is held to what this clone derives, as `replay` holds a root to its nodes.
fn disagreement(r: &Release, d: &Derived<'_>) -> Option<String> {
    let sorted = |v: &[Sha256Digest]| {
        let mut v = v.to_vec();
        v.sort();
        v
    };
    let waived = |v: &[jtac_core::GitOid]| {
        let mut v = v.to_vec();
        v.sort();
        v
    };
    let declared_off = d.verdict.waived.len() as u64;
    if r.object != d.tag.object || r.commit != d.tag.head.commit || r.tree != d.tag.head.tree {
        Some(format!(
            "it attests {} at {}, not this tag",
            r.commit, r.object
        ))
    } else if r.from.commit != d.from.commit {
        Some(format!(
            "its range starts after {}, not {}",
            r.from.commit, d.from.commit
        ))
    } else if r.ci != d.require_ci {
        Some(format!(
            "it was written {} `--require-ci`",
            if r.ci { "with" } else { "without" }
        ))
    } else if !d.verdict.pass {
        Some("this run refuses the range it attests".to_owned())
    } else if r.commits != d.commits
        || r.declared_off != declared_off
        || r.covered != d.commits - declared_off
    {
        Some(format!(
            "it counts {} commits, {} covered, {} declared off; this run {}, {}, {}",
            r.commits,
            r.covered,
            r.declared_off,
            d.commits,
            d.commits - declared_off,
            declared_off
        ))
    } else if waived(&r.waived) != waived(&d.verdict.waived) {
        Some("the commits it lists as declared off are not the ones this run finds".to_owned())
    } else if sorted(&r.roots) != sorted(&d.verdict.roots) {
        Some("the roots it names are not the roots this clone judges the range by".to_owned())
    } else if !d.artifacts.is_empty() && r.artifacts != d.artifacts {
        Some("the artifacts it names are not the ones built now".to_owned())
    } else {
        None
    }
}

fn push(store: &Store, remote: &str, tag: &str) -> Result<String, Refusal> {
    let pushed = store.push_release(remote, tag).map_err(Refusal::store)?;
    Ok(match pushed.outcome {
        PushOutcome::Sent => "sent".to_owned(),
        PushOutcome::UpToDate => "up to date".to_owned(),
        PushOutcome::Rejected { summary } => format!("rejected: {summary}"),
    })
}
