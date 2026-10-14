//! The decision function for git operations (DESIGN.md section 11.3) and, in M10b, the shell
//! command classifier. Pure: reads no disk, runs no process.

pub mod push;

pub use push::{Coverage, Decision, History, Pass, RefUpdate, Refuse, decide, exempt};
