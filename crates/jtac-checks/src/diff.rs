//! Diff-integrity checks (DESIGN.md section 10.3): pure rules over the files a change touched.
//! `git` gathers the changes; `report` decides. Findings here carry no check id: `jtac check`
//! writes the `integrity` check node and points them at it.

mod add;
mod attr;
mod change;
mod ci;
mod cmt;
pub mod git;
mod lang;
mod lines;
mod prot;
mod report;
mod rule;
mod scan;
mod sec;
mod size;
mod span;
mod sup;
mod test;
mod view;

pub use change::{Blob, Change, Edit};
pub use lang::Lang;
pub use prot::Adoption;
pub use report::{Finding, Report, report};
pub use rule::Rule;
