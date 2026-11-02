//! Evidence types, canonical encoding, hashing, and the Merkle tree (DESIGN.md section 9).
//!
//! Pure: no I/O, no clock, no process.

pub mod canonical;
pub mod compare;
pub mod counts;
pub mod digest;
pub mod from;
pub mod merkle;
pub mod node;
pub mod oid;
pub mod root;
pub mod schema;
pub mod signer;
pub mod time;
pub mod verdict;
pub mod waiver;

pub use canonical::{CanonicalError, ReadError, canonical_bytes, canonical_digest, from_canonical};
pub use compare::{Compared, Run, compare};
pub use counts::counts;
pub use digest::{ParseError, Sha256Digest};
pub use from::FromHead;
pub use merkle::{InclusionProof, MerkleError, merkle_root, prove};
pub use node::{
    Action, Body, Capture, CheckBody, FindingBody, Gate, Kind, LeverBody, Node, Outcome, Scope,
    Severity,
};
pub use oid::GitOid;
pub use root::{
    Counts, Covers, FindingCounts, GateRef, Parent, PolicyRef, PolicySource, Root, RootError,
    SignedLever, State,
};
pub use schema::{NodeVersion, RootVersion, V1};
pub use signer::Signer;
pub use time::{Ts, TsError};
pub use verdict::{CheckSpec, Facts, Reason, Verdict, gate};
pub use waiver::{MAX_WAIVER_SECS, OutOfScope, Waivable, clean_prefix, covers_finding, scope_at};
