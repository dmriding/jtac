//! The stored waivers a verdict may apply (DESIGN.md section 11.6): each one's signature
//! must hold over its node in `jtac-lever-v1` and name its signer, or the writer would sign a
//! root carrying a waiver nobody signed, which every reader then rejects. Trust is the
//! reader's question; the writer asks only whether the signature is real.

use std::path::Path;

use jtac_core::SignedLever;
use jtac_sign::{Namespace, Trust, Verified};
use jtac_store::Store;

use crate::coverage;
use crate::refuse::Refusal;

/// Every stored waiver whose signature holds; one that does not is a note, never offered.
pub fn offered(
    repo: &Path,
    store: &Store,
    notes: &mut Vec<String>,
) -> Result<Vec<SignedLever>, Refusal> {
    let stored = store.levers().map_err(Refusal::store)?;
    if stored.is_empty() {
        return Ok(stored);
    }
    let tool = coverage::verifier(repo)?;
    let mut out = Vec::new();
    for lever in stored {
        let jtac_core::Body::Lever(body) = &lever.node.body else {
            continue;
        };
        let bytes = lever.node.canonical_bytes().map_err(Refusal::internal)?;
        let verified = jtac_sign::verify(
            &tool,
            Trust::None,
            &body.signer,
            Namespace::Lever,
            lever.node.ts,
            &bytes,
            &lever.sig,
        )
        .map_err(|e| Refusal::lever_verifier(&e))?;
        match verified {
            Verified::Invalid { what } => notes.push(format!(
                "waiver {} is not applied: its signature does not hold ({what}); remove `{}` \
                 and write it again with `jtac waive`",
                lever.node.id().map_err(Refusal::internal)?,
                store
                    .lever_path(&lever.node.id().map_err(Refusal::internal)?)
                    .display()
            )),
            Verified::Trusted | Verified::Unknown { .. } => out.push(lever),
        }
    }
    Ok(out)
}
