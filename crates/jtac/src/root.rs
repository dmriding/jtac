//! Assemble the root from what was judged (DESIGN.md section 9.4). Pure: every field comes
//! in as an argument, and `counts` and the Merkle root are the shared implementations replay
//! recomputes with.

use jtac_core::{
    CanonicalError, GateRef, MerkleError, Node, PolicyRef, PolicySource, Root, Sha256Digest,
    Signer, V1, counts, merkle_root,
};

use crate::chain::Chain;
use crate::judge::Judged;

/// Why the root could not be assembled: a node with no canonical form, or a repeated one.
#[derive(Debug, thiserror::Error)]
pub enum BuildError {
    /// See [`CanonicalError`].
    #[error(transparent)]
    Canonical(#[from] CanonicalError),
    /// See [`MerkleError`].
    #[error(transparent)]
    Merkle(#[from] MerkleError),
}

/// The root and its leaves, the node ids in `nodes` order. No lever leaves in this release,
/// so `levers` is empty and a `declared_off` verdict is refused by `Root::validate`.
pub fn build(
    j: &Judged,
    chain: Chain,
    policy_version: u32,
    gate: GateRef,
    signer: Signer,
) -> Result<(Root, Vec<Sha256Digest>), BuildError> {
    let leaves: Vec<Sha256Digest> = j.nodes.iter().map(Node::id).collect::<Result<_, _>>()?;
    let evidence = merkle_root(&leaves)?;
    let root = Root {
        v: V1,
        commit: j.head.commit,
        tree: j.head.tree,
        ts: j.now,
        covers: chain.covers,
        parents: chain.parents,
        policy: PolicyRef {
            hash: j.facts.policy,
            version: policy_version,
            source: PolicySource::Repo,
        },
        gate,
        state: j.verdict.state,
        evidence,
        levers: Vec::new(),
        counts: counts(&j.nodes),
        signer,
    };
    Ok((root, leaves))
}
