//! Evidence types, canonical encoding, hashing, and the Merkle tree (DESIGN.md section 9).
//!
//! Pure: no I/O, no clock, no process. The verdict function lands here in M3.

pub mod canonical;
pub mod digest;
pub mod merkle;
pub mod node;
pub mod oid;
pub mod root;
pub mod schema;
pub mod signer;

pub use canonical::{CanonicalError, canonical_bytes, canonical_digest};
pub use digest::{ParseError, Sha256Digest};
pub use jiff::Timestamp;
pub use merkle::{InclusionProof, MerkleError, merkle_root, prove};
pub use node::{
    Action, Body, CheckBody, FindingBody, Kind, LeverBody, Node, Outcome, Scope, Severity,
};
pub use oid::GitOid;
pub use root::{
    Counts, Covers, FindingCounts, GateRef, Parent, PolicyRef, PolicySource, Root, RootError, State,
};
pub use schema::V1;
pub use signer::Signer;
