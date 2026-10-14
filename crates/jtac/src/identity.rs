//! Who signs and what signed (DESIGN.md section 11.8): the key git config names, resolved
//! before any check runs so a missing key costs nothing, and the digest of this binary.

use std::path::Path;

use jtac_core::{GateRef, Sha256Digest};
use jtac_sign::{GitConfig, Identity, KeyRef, resolve};

use crate::refuse::Refusal;
use crate::repo::config;

/// The signing identity git config names for `repo`. A relative key path is read from the
/// repository root, where git reads it, not from where `jtac` was run.
pub fn identity(repo: &Path) -> Result<Identity, Refusal> {
    let get = |key: &str| config(repo, key).map_err(|e| Refusal::repository(&e));
    let cfg = GitConfig {
        jtac_signingkey: get("jtac.signingkey")?,
        user_signingkey: get("user.signingkey")?,
        gpg_format: get("gpg.format")?,
        gpg_ssh_program: get("gpg.ssh.program")?,
        user_email: get("user.email")?,
    };
    let mut resolved = resolve(&cfg).map_err(|e| Refusal::no_identity(&e))?;
    if let KeyRef::File(path) = &resolved.key
        && path.is_relative()
        && !path.to_string_lossy().starts_with('~')
    {
        resolved.key = KeyRef::File(repo.join(path));
    }
    jtac_sign::identity(resolved).map_err(|e| Refusal::sign(&e))
}

/// This binary's version and digest, what the root names as its gate.
pub fn gate_ref() -> Result<GateRef, Refusal> {
    let exe = std::env::current_exe().map_err(|e| Refusal::exe(&e))?;
    let bytes = std::fs::read(&exe).map_err(|e| Refusal::exe(&e))?;
    Ok(GateRef {
        version: env!("CARGO_PKG_VERSION").to_owned(),
        binary: Sha256Digest::of(bytes),
    })
}
