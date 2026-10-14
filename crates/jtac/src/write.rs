//! What `jtac check` leaves in the store (DESIGN.md section 11.1 steps 6 and 7): nodes and
//! captures on every verdict, then the cache entries that name them, the signed root on a pass.

use jtac_checks::Cache;
use jtac_core::{Root, Sha256Digest};
use jtac_sign::{Identity, Namespace, sign};
use jtac_store::{Store, StoreError};

use crate::judge::Judged;
use crate::refuse::Refusal;

/// Keep every capture and node, and only then each fresh pass in the cache, so an interrupted
/// run never leaves a cache entry whose evidence is missing.
pub fn evidence(store: &Store, j: &Judged, cache: &mut dyn Cache) -> Result<(), Refusal> {
    for bytes in &j.captures {
        store.put_blob(bytes).map_err(Refusal::store)?;
    }
    for node in &j.nodes {
        store.put_node(node).map_err(Refusal::store)?;
    }
    for node in &j.nodes {
        jtac_checks::store(cache, node).map_err(Refusal::store)?;
    }
    Ok(())
}

/// Sign `root` over its canonical bytes and create its ref. Returns the root id, or `None`
/// when another run created the tip's ref first (issue 27): the caller reports that root.
pub fn root(
    store: &Store,
    id: &Identity,
    root: &Root,
    leaves: &[Sha256Digest],
) -> Result<Option<Sha256Digest>, Refusal> {
    let bytes = root.canonical_bytes().map_err(Refusal::internal)?;
    let sig = sign(id, Namespace::Root, &bytes).map_err(|e| Refusal::sign(&e))?;
    match store.put_root(root, sig.as_bytes(), leaves) {
        Ok(id) => Ok(Some(id)),
        Err(StoreError::Exists { .. }) => Ok(None),
        Err(e) => Err(Refusal::store(e)),
    }
}
