//! `cache/<key name hex>`: the file-backed check cache. An entry holds the passing node's
//! canonical bytes; `jtac_checks::lookup` re-checks the key, so a wrong file is a rerun.

use std::path::PathBuf;

use jtac_checks::{Cache, CacheError};
use jtac_core::{Node, Sha256Digest, from_canonical};

use crate::fs;

/// The cache under one store.
#[derive(Clone, Debug)]
pub struct FileCache {
    dir: PathBuf,
}

impl FileCache {
    pub(crate) fn new(dir: PathBuf) -> Self {
        Self { dir }
    }

    fn path(&self, name: &Sha256Digest) -> PathBuf {
        self.dir.join(fs::hex(name))
    }
}

impl Cache for FileCache {
    fn get(&self, name: &Sha256Digest) -> Option<Node> {
        let bytes = std::fs::read(self.path(name)).ok()?;
        from_canonical(&bytes).ok()
    }

    fn put(&mut self, name: Sha256Digest, node: Node) -> Result<(), CacheError> {
        let path = self.path(&name);
        fs::write(&path, &node.canonical_bytes()?).map_err(|e| match e {
            crate::StoreError::Io { path, source } => CacheError::Io { path, source },
            other => CacheError::Io {
                path,
                source: std::io::Error::other(other.to_string()),
            },
        })
    }
}
