//! SSHSIG signing and verification through ssh-keygen (DESIGN.md section 11.8).
//!
//! Impure: gathers facts and decides nothing. ssh-keygen does every cryptographic operation;
//! this crate names the key, the namespace and the trust file, and reads the envelope.

mod fingerprint;
mod identity;
mod key;
mod namespace;
mod sign;
mod tool;
mod verify;

use std::path::PathBuf;

pub use identity::{Identity, identity, verifier};
pub use key::{GitConfig, KeyRef, ResolveError, Resolved, resolve};
pub use namespace::Namespace;
pub use sign::sign;
pub use tool::Tool;
pub use verify::{TRUST_CAP, Trust, Verified, verify, verify_time};

/// Why signing or verifying could not run; a signature that fails is a [`Verified`].
#[derive(Debug, thiserror::Error)]
pub enum SignError {
    /// ssh-keygen could not be started.
    #[error("could not run `{}`: {source}", argv.join(" "))]
    Spawn {
        /// The command line.
        argv: Vec<String>,
        /// The OS error.
        source: std::io::Error,
    },
    /// ssh-keygen ran and refused.
    #[error("`{}` failed: {stderr}", argv.join(" "))]
    Failed {
        /// The command line.
        argv: Vec<String>,
        /// The first line of stderr.
        stderr: String,
    },
    /// A file could not be read or written.
    #[error("{}: {source}", path.display())]
    Io {
        /// The file.
        path: PathBuf,
        /// The OS error.
        source: std::io::Error,
    },
    /// ssh-keygen could not read the key.
    #[error("{}: {what}", path.display())]
    Key {
        /// The file.
        path: PathBuf,
        /// What was wrong.
        what: String,
    },
    /// A trust file that is not a plain file of a sane size.
    #[error("{}: {what}", path.display())]
    Trust {
        /// The file.
        path: PathBuf,
        /// What was wrong.
        what: &'static str,
    },
    /// ssh-keygen signed, but not as the identity claims.
    #[error("the signature was made with key {found}, not {expected}")]
    KeyMismatch {
        /// The identity's fingerprint.
        expected: String,
        /// The key ssh-keygen named, or nothing readable.
        found: String,
    },
}
