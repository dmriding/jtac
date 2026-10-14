//! `jtac hook pre-push` (DESIGN.md section 11.3): git's stdin lines to ref updates, the facts
//! around each, the pure decision, then the root refs pushed ahead of the code. Nothing here
//! writes to stdout; a hook that cannot answer refuses.

use std::path::Path;

use crate::refuse::Refusal;

/// What the hook wrote to stderr and how it exited: 0 lets the push proceed, 1 refuses it,
/// 2 could not answer, which also refuses.
pub fn pre_push(_cwd: &Path, _remote: &str, _url: &str, _stdin: &str) -> (String, u8) {
    (Refusal::not_built("hook pre-push").text(), 2)
}
