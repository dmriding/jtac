//! `jtac init` (DESIGN.md section 11.7): remote, key, store, policy, hook, refspec, lever
//! node. `--dry-run` prints the same plan and writes nothing; a second run finds nothing to
//! do.

use std::path::Path;

use crate::Printed;
use crate::cli::InitArgs;
use crate::refuse::Refusal;

/// Set the repository up, or say what would be.
pub fn init(_a: &InitArgs, _cwd: &Path) -> Result<Printed, Refusal> {
    Err(Refusal::not_built("init"))
}
