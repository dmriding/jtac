//! Assemble the root from what was judged (DESIGN.md section 9.4). Pure: every field comes
//! in as an argument, and `counts` and the Merkle root are the shared implementations replay
//! recomputes with.

use jtac_core::{
    CanonicalError, FromHead, GateRef, GitOid, MerkleError, Node, PolicyRef, PolicySource, Root,
    RootVersion, Sha256Digest, Signer, counts, merkle_root,
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
    /// The base git named is not an object id in HEAD's format.
    #[error("the base `{0}` is not an object id")]
    Base(String),
}

/// The root and its leaves, the node ids in `nodes` order, the applied waivers as `levers`.
/// A base at HEAD is a parentless tip judged against itself (DESIGN.md section 10.3), so the
/// root names none. `from` is the merge job's head (section 15), `None` for a local root.
pub fn build(
    j: &Judged,
    chain: Chain,
    policy_version: u32,
    gate: GateRef,
    signer: Signer,
    from: Option<FromHead>,
) -> Result<(Root, Vec<Sha256Digest>), BuildError> {
    let leaves: Vec<Sha256Digest> = j.nodes.iter().map(Node::id).collect::<Result<_, _>>()?;
    let evidence = merkle_root(&leaves)?;
    let base = (j.base.commit != j.head.commit.hex())
        .then(|| oid_like(&j.head.commit, &j.base.commit))
        .transpose()?;
    let root = Root {
        v: RootVersion::V2,
        commit: j.head.commit,
        tree: j.head.tree,
        ts: j.now,
        covers: chain.covers,
        parents: chain.parents,
        base,
        from,
        policy: PolicyRef {
            hash: j.facts.policy,
            version: policy_version,
            source: PolicySource::Repo,
        },
        gate,
        state: j.verdict.state,
        evidence,
        levers: j.levers.clone(),
        counts: counts(&j.nodes),
        signer,
    };
    Ok((root, leaves))
}

fn oid_like(like: &GitOid, hex: &str) -> Result<GitOid, BuildError> {
    let prefix = match like {
        GitOid::Sha1(_) => "sha1",
        GitOid::Sha256(_) => "sha256",
    };
    format!("{prefix}:{hex}")
        .parse()
        .map_err(|_| BuildError::Base(hex.to_owned()))
}
