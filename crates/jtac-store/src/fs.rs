//! Atomic file writes and hash-checked reads. Every store file is written whole into a temp
//! file beside its final name and renamed over it, so a reader never sees a partial file.

use std::path::Path;

use jtac_core::Sha256Digest;

use crate::StoreError;

/// Write `bytes` at `path`, creating parents, replacing whatever was there.
pub(crate) fn write(path: &Path, bytes: &[u8]) -> Result<(), StoreError> {
    let parent = path.parent().expect("store paths have a parent");
    std::fs::create_dir_all(parent).map_err(StoreError::io(parent))?;
    let tmp = tempfile::Builder::new()
        .prefix(".tmp-")
        .tempfile_in(parent)
        .map_err(StoreError::io(parent))?;
    std::fs::write(tmp.path(), bytes).map_err(StoreError::io(tmp.path()))?;
    tmp.persist(path)
        .map(|_| ())
        .map_err(|e| StoreError::io(path)(e.error))
}

/// The file at `path`, or `None` when there is none.
pub(crate) fn read(path: &Path) -> Result<Option<Vec<u8>>, StoreError> {
    match std::fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(StoreError::io(path)(e)),
    }
}

/// [`read`], refusing a file whose bytes do not hash to `want`.
pub(crate) fn read_hashed(path: &Path, want: &Sha256Digest) -> Result<Option<Vec<u8>>, StoreError> {
    let Some(bytes) = read(path)? else {
        return Ok(None);
    };
    if Sha256Digest::of(&bytes) != *want {
        return Err(StoreError::corrupt(
            path,
            "the bytes do not hash to the name",
        ));
    }
    Ok(Some(bytes))
}

/// The hex of a digest, the file name it is stored under.
pub(crate) fn hex(digest: &Sha256Digest) -> String {
    digest.to_string()[Sha256Digest::PREFIX.len()..].to_owned()
}
