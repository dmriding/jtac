//! The decision function for git operations (DESIGN.md section 11.3), the coverage state
//! of a range of commits (section 9.5) and, in M10b, the shell command classifier. Pure:
//! reads no disk, runs no process.

pub mod audit;
pub mod push;

pub use audit::{CommitFact, RootFact, Row, Trust, Via, rows};
pub use push::{Coverage, Decision, History, Pass, RefUpdate, Refuse, decide, exempt};
