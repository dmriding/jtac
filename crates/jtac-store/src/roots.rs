//! `roots/<root id hex>/`: the root, its signature, and the local-only manifest of the node
//! ids that are its Merkle leaves, which `jtac replay` needs and the root does not carry.

use std::path::PathBuf;

use jtac_core::{GitOid, Root, Sha256Digest, V1, canonical_bytes, from_canonical};
use serde::{Deserialize, Serialize};

use crate::{Store, StoreError, fs, refs};

/// The leaves of one root's Merkle tree. Never leaves the machine.
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    v: V1,
    nodes: Vec<Sha256Digest>,
}

/// A root with its signature as stored.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StoredRoot {
    /// The root.
    pub root: Root,
    /// The detached SSHSIG over its canonical bytes, as given to `put_root`.
    pub sig: Vec<u8>,
}

impl Store {
    /// Point the root's ref at a commit holding `root.json` and `root.sig`, then keep the
    /// local copy with the ids of its nodes. Returns the root id. The ref goes first so a
    /// refused root leaves no local copy behind.
    pub fn put_root(
        &self,
        root: &Root,
        sig: &[u8],
        nodes: &[Sha256Digest],
    ) -> Result<Sha256Digest, StoreError> {
        let id = root.id()?;
        let dir = self.root_dir(&id);
        let manifest = Manifest {
            v: V1,
            nodes: nodes.to_vec(),
        };
        refs::write(self, root, sig)?;
        fs::write(&dir.join("root.json"), &root.canonical_bytes()?)?;
        fs::write(&dir.join("root.sig"), sig)?;
        fs::write(&dir.join("nodes.json"), &canonical_bytes(&manifest)?)?;
        Ok(id)
    }

    /// The root for `commit` from its ref, or `None` when the ref does not exist. The root
    /// is refused unless its bytes are canonical, its shape validates, and it names `commit`.
    pub fn root(&self, commit: &GitOid) -> Result<Option<StoredRoot>, StoreError> {
        refs::read(self, commit)
    }

    /// The local copy of the root with `id`, or `None`. A fetched root has no local copy.
    pub fn local_root(&self, id: &Sha256Digest) -> Result<Option<StoredRoot>, StoreError> {
        let dir = self.root_dir(id);
        let json = dir.join("root.json");
        let Some(bytes) = fs::read_hashed(&json, id)? else {
            return Ok(None);
        };
        let root: Root = from_canonical(&bytes).map_err(|e| StoreError::unreadable(&json, e))?;
        root.validate().map_err(|e| StoreError::corrupt(&json, e))?;
        let sig_path = dir.join("root.sig");
        let sig = fs::read(&sig_path)?
            .ok_or_else(|| StoreError::corrupt(&sig_path, "the signature file is missing"))?;
        Ok(Some(StoredRoot { root, sig }))
    }

    /// The node ids of the root with `id`, or `None` when the root was not written here.
    pub fn manifest(&self, id: &Sha256Digest) -> Result<Option<Vec<Sha256Digest>>, StoreError> {
        let path = self.root_dir(id).join("nodes.json");
        fs::read(&path)?
            .map(|bytes| {
                from_canonical::<Manifest>(&bytes)
                    .map(|m| m.nodes)
                    .map_err(|e| StoreError::unreadable(&path, e))
            })
            .transpose()
    }

    fn root_dir(&self, id: &Sha256Digest) -> PathBuf {
        self.dir.join("roots").join(fs::hex(id))
    }
}
