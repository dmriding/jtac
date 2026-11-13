//! Parse, validate, hash, and evaluate policy ([DESIGN.md] section 10).
//!
//! Pure: takes policy text and facts as arguments. See [DESIGN.md] section 8.1.
//!
//! [DESIGN.md]: https://github.com/dmriding/jtac/blob/main/DESIGN.md

pub mod defaults;
mod echo;
pub mod error;
pub mod facts;
pub mod glob;
pub mod hash;
pub mod parse;
pub mod schema;
pub mod validate;

pub use defaults::{Detected, Js, Runner, default_policy};
pub use error::PolicyError;
pub use facts::facts;
pub use glob::{Matcher, Matchers};
pub use hash::{policy_hash, to_toml};
pub use parse::parse;
pub use schema::{
    Check, DEFAULT_TIMEOUT_SECS, FenceMode, Fences, INTEGRITY_CHECK_ID, Integrity, JTAC_DIR_GLOB,
    Policy, Protected, Scope, Tier,
};
pub use validate::validate;

/// Where v0.1 reads policy, relative to the repository root.
pub const POLICY_PATH: &str = ".jtac/policy.toml";
