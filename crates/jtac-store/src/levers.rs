//! `levers/<node id hex>.sig`: the signature over a lever node whose bytes are in `objects/`.
//! A waiver is local until a verdict applies it; `check` reads every one here and the verdict
//! decides which are in scope (DESIGN.md section 11.6).

use std::path::PathBuf;

use jtac_core::{Node, Sha256Digest, SignedLever};

use crate::{Store, StoreError, fs};

impl Store {
    /// Keep `node` and the signature over its canonical bytes. Returns the node id.
    pub fn put_lever(&self, node: &Node, sig: &str) -> Result<Sha256Digest, StoreError> {
        let id = self.put_node(node)?;
        fs::write(&self.lever_path(&id), sig.as_bytes())?;
        Ok(id)
    }

    /// Every lever with a signature here, by node id. A signature file whose node is missing
    /// or whose name is not a digest is corrupt: a waiver nobody can read must not vanish.
    pub fn levers(&self) -> Result<Vec<SignedLever>, StoreError> {
        let dir = self.dir.join("levers");
        let entries = match std::fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(StoreError::io(&dir)(e)),
        };
        let mut names: Vec<PathBuf> = Vec::new();
        for entry in entries {
            names.push(entry.map_err(StoreError::io(&dir))?.path());
        }
        names.sort();
        let mut out = Vec::new();
        for path in names {
            let Some(hex) = path
                .file_name()
                .and_then(|n| n.to_str())
                .and_then(|n| n.strip_suffix(".sig"))
            else {
                return Err(StoreError::corrupt(&path, "not a lever signature file"));
            };
            let id: Sha256Digest = format!("{}{hex}", Sha256Digest::PREFIX)
                .parse()
                .map_err(|_| StoreError::corrupt(&path, "the name is not a digest"))?;
            let node = self
                .node(&id)?
                .ok_or_else(|| StoreError::corrupt(&path, "its node is not in the store"))?;
            let sig = fs::read(&path)?
                .ok_or_else(|| StoreError::corrupt(&path, "the signature file is missing"))?;
            let sig = String::from_utf8(sig)
                .map_err(|_| StoreError::corrupt(&path, "the signature is not UTF-8"))?;
            out.push(SignedLever { node, sig });
        }
        Ok(out)
    }

    /// Where the signature of lever `id` lives: the file a person removes to withdraw an
    /// unapplied waiver.
    pub fn lever_path(&self, id: &Sha256Digest) -> PathBuf {
        self.dir.join("levers").join(format!("{}.sig", fs::hex(id)))
    }
}
