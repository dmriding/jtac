//! The failure record (DESIGN.md section 15.2, decisions 56 and 66): on a failed verdict
//! with every result read, the root the job would have written, with the state `failed`,
//! at `refs/jtac/failed/`, created once. A record already on the tip is judged here and by
//! no verdict that passes: CI's own for this head is kept, any other stops the job with
//! its ref named.

use jtac_core::{FromHead, GitOid, Sha256Digest};
use jtac_sign::{Namespace, sign};
use jtac_store::{Space, StoreError};

use super::Keeper;
use crate::chain::Chain;
use crate::check::Site;
use crate::judge::Judged;
use crate::refuse::Refusal;
use crate::root;

/// The record the tip holds after a failed verdict.
pub struct Recorded {
    /// Its id.
    pub id: Sha256Digest,
    /// This run wrote it.
    pub fresh: bool,
    /// What a record that already stood says of the head.
    pub stood: Option<FromHead>,
}

/// Write the record, or keep the one that stands; `chain` is computed only to write.
pub fn record(
    site: &Site,
    keeper: &Keeper,
    j: &Judged,
    chain: impl FnOnce() -> Result<Chain, Refusal>,
    notes: &mut Vec<String>,
) -> Result<Recorded, Refusal> {
    let tip = &j.head.commit;
    if let Some(found) = standing(site, keeper, tip, notes)? {
        return Ok(found);
    }
    let (root, leaves) = root::build(
        j,
        chain()?,
        site.policy_version,
        site.gate.clone(),
        site.identity.signer.clone(),
        site.from.clone(),
    )
    .map_err(Refusal::internal)?;
    let bytes = root.canonical_bytes().map_err(Refusal::internal)?;
    let sig =
        sign(site.identity, Namespace::of_root(&root), &bytes).map_err(|e| Refusal::sign(&e))?;
    match site.store.put_record(&root, sig.as_bytes(), &leaves) {
        Ok(id) => Ok(Recorded {
            id,
            fresh: true,
            stood: None,
        }),
        Err(StoreError::Exists { .. }) => standing(site, keeper, tip, notes)?
            .ok_or_else(|| Refusal::internal("the tip's failure record ref vanished")),
        Err(e) => Err(Refusal::store(e)),
    }
}

/// The record already on the tip, kept as CI's own for this head or refused.
fn standing(
    site: &Site,
    keeper: &Keeper,
    tip: &GitOid,
    notes: &mut Vec<String>,
) -> Result<Option<Recorded>, Refusal> {
    let Some(stored) = keeper.held(site.store, Space::Failed)? else {
        return Ok(None);
    };
    if let Some(why) = keeper.reason(&stored)? {
        return Err(Refusal::standing(Space::Failed, tip, &why, keeper.remote));
    }
    notes.push(
        "the tip already holds CI's own failure record for this head; it stands, and this run \
         wrote none"
            .to_owned(),
    );
    Ok(Some(Recorded {
        id: stored.root.id().map_err(Refusal::internal)?,
        fresh: false,
        stood: stored.root.from,
    }))
}
