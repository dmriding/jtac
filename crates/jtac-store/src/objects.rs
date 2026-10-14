//! `objects/<sha256 hex>`: nodes as canonical bytes and captured output as raw bytes, both
//! named by the digest of the bytes stored.

use std::path::PathBuf;

use jtac_core::{Node, Sha256Digest, from_canonical};

use crate::{Store, StoreError, fs};

impl Store {
    /// Keep `node`. Returns its id.
    pub fn put_node(&self, node: &Node) -> Result<Sha256Digest, StoreError> {
        self.put_blob(&node.canonical_bytes()?)
    }

    /// The node with `id`, or `None` when it is not here.
    pub fn node(&self, id: &Sha256Digest) -> Result<Option<Node>, StoreError> {
        let path = self.object_path(id);
        fs::read_hashed(&path, id)?
            .map(|bytes| from_canonical(&bytes).map_err(|e| StoreError::unreadable(&path, e)))
            .transpose()
    }

    /// Keep `bytes` as captured output. Returns its digest.
    pub fn put_blob(&self, bytes: &[u8]) -> Result<Sha256Digest, StoreError> {
        let digest = Sha256Digest::of(bytes);
        fs::write(&self.object_path(&digest), bytes)?;
        Ok(digest)
    }

    /// The captured output with `digest`, or `None` when it is not here.
    pub fn blob(&self, digest: &Sha256Digest) -> Result<Option<Vec<u8>>, StoreError> {
        fs::read_hashed(&self.object_path(digest), digest)
    }

    fn object_path(&self, digest: &Sha256Digest) -> PathBuf {
        self.dir.join("objects").join(fs::hex(digest))
    }
}
