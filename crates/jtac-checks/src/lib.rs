//! Check runner: spawn a policy check, cap its output, kill it on time, mark cache hits
//! (DESIGN.md sections 11.1 and 8.1). Impure: gathers facts and decides nothing.
//! Diff-integrity checks land in M5.

mod cache;
mod env;
mod run;
mod spec;
mod version;

pub use cache::{Cache, CacheKey, MemoryCache, lookup, store};
pub use env::{allowed, scrub};
pub use run::{CAPTURE_CAP, Context, Ran, run};
pub use spec::{Spec, specs};
pub use version::{PROBE_TIMEOUT, VERSION_CAP, first_line, probe_argv};
