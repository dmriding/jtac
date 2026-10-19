//! The decision function for git operations (DESIGN.md section 11.3), the coverage state
//! of a range of commits (section 9.5) and the shell command classifier (section 11.4).
//! Pure: reads no disk, runs no process.

pub mod advise;
pub mod audit;
pub mod classify;
pub mod push;
mod shell;

pub use advise::note;
pub use audit::{CommitFact, RootFact, Row, Trust, Via, rows};
pub use push::{Coverage, Decision, History, Pass, RefUpdate, Refuse, decide, exempt};
